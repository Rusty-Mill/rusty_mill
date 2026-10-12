//! Runs a simulated RLBot core on `rb_env`'s physics, so RLBot v5 bots can play a match with
//! no Rocket League installed.
//!
//! ```text
//! rb_rlbot_core [--port N] [--lockstep] [--length five|ten|twenty|unlimited] AGENT_ID:TEAM...
//! ```
//!
//! Each `AGENT_ID:TEAM` adds one car driven by the bot that connects with that agent id (give
//! the same id twice for a hivemind's two cars). The match starts when every named bot has
//! connected and sent `InitComplete`. Start the bots yourself with `RLBOT_SERVER_PORT` set to
//! the port printed here, e.g. `RLBOT_AGENT_ID=rusty_bullet/chaser_bot rb_chaser_bot`.
//! `--lockstep` makes the run repeatable (see `Pacing`). Ctrl-C ends it.

use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use rb_rlbot_core::{bot_match, Core, Event, Pacing, Simulation};
use rb_rlbot_wire::MatchLengthMutator;

const DEFAULT_PORT: u16 = 23234;
const USAGE: &str = "usage: rb_rlbot_core [--port N] [--lockstep] \
                     [--length five|ten|twenty|unlimited] AGENT_ID:TEAM...";

struct Args {
    port: u16,
    pacing: Pacing,
    length: MatchLengthMutator,
    players: Vec<(String, u32)>,
}

fn parse(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut parsed = Args {
        port: DEFAULT_PORT,
        pacing: Pacing::RealTime,
        length: MatchLengthMutator::FiveMinutes,
        players: Vec::new(),
    };
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                let value = args.next().ok_or("--port needs a value")?;
                parsed.port = value
                    .parse()
                    .map_err(|_| format!("--port: {value:?} is not a port"))?;
            }
            "--lockstep" => {
                parsed.pacing = Pacing::Lockstep {
                    input_timeout: Duration::from_secs(5),
                };
            }
            "--length" => {
                let value = args.next().ok_or("--length needs a value")?;
                parsed.length = match value.as_str() {
                    "five" => MatchLengthMutator::FiveMinutes,
                    "ten" => MatchLengthMutator::TenMinutes,
                    "twenty" => MatchLengthMutator::TwentyMinutes,
                    "unlimited" => MatchLengthMutator::Unlimited,
                    other => return Err(format!("--length: unknown length {other:?}")),
                };
            }
            player => {
                // The team is after the last colon: agent ids may contain none, but be safe.
                let (id, team) = player
                    .rsplit_once(':')
                    .ok_or_else(|| format!("{player:?} is not AGENT_ID:TEAM"))?;
                let team: u32 = team
                    .parse()
                    .map_err(|_| format!("{player:?}: the team must be 0 or 1"))?;
                if id.trim().is_empty() {
                    return Err(format!("{player:?}: the agent id is empty"));
                }
                parsed.players.push((id.to_string(), team));
            }
        }
    }
    Ok(parsed)
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse(std::env::args().skip(1)).map_err(|e| format!("{e}\n{USAGE}"))?;
    let sim = Simulation::new(bot_match(&args.players, args.length))?;
    let mut core = Core::bind(("127.0.0.1", args.port), sim, args.pacing)?;
    // Ctrl-C ends the process directly: the core holds no state worth saving.
    let stop = AtomicBool::new(false);
    let end = core.run(&stop, |event| match event {
        Event::Listening(addr) => println!("rb_rlbot_core listening on {addr}"),
        event => println!("{event:?}"),
    })?;
    println!("rb_rlbot_core finished: {end:?}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rb_rlbot_core: {e}");
            ExitCode::FAILURE
        }
    }
}

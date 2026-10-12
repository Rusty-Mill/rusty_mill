//! RLBot's middleman job against the real game: Rocket League on one side, RLBot v5 bots on
//! the other.
//!
//! ```text
//! rb_rl_bridge [--game-port N] [--port N] [--launch RocketLeague.exe] [--no-spawn] AGENT_ID:TEAM...
//! ```
//!
//! Listens for the game on `--game-port` (default 23233) and serves bots on `--port` (default
//! 23234). Start the game with the arguments printed here (or pass `--launch`) and a match
//! loading. Once the map is up the bridge spawns one bot car per `AGENT_ID:TEAM`, in that order,
//! as RLBot does; with `--no-spawn` it expects those cars to exist already. Then start the bots
//! with `RLBOT_SERVER_PORT` set. Never run against a live game as of its writing;
//! `rb_rl_game_probe` is the first thing to try.

use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use rb_rl_game_bridge::{
    rlbot_args, GameLaunch, GameListener, LinkBackend, IDEAL_GAME_PORT, RLBOT_SOCKETS_PORT,
};
use rb_rlbot_core::{bot_match, Core, Event, Pacing};
use rb_rlbot_wire::MatchLengthMutator;

struct Args {
    game_port: u16,
    port: u16,
    launch: Option<String>,
    spawn: bool,
    players: Vec<(String, u32)>,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut parsed = Args {
        game_port: IDEAL_GAME_PORT,
        port: RLBOT_SOCKETS_PORT,
        launch: None,
        spawn: true,
        players: Vec::new(),
    };
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--game-port" => {
                parsed.game_port = value("--game-port")?
                    .parse()
                    .map_err(|_| "--game-port: not a port".to_string())?;
            }
            "--port" => {
                parsed.port = value("--port")?
                    .parse()
                    .map_err(|_| "--port: not a port".to_string())?;
            }
            "--launch" => parsed.launch = Some(value("--launch")?),
            "--no-spawn" => parsed.spawn = false,
            player => {
                let (id, team) = player
                    .rsplit_once(':')
                    .ok_or_else(|| format!("{player:?} is not AGENT_ID:TEAM"))?;
                let team: u32 = team
                    .parse()
                    .map_err(|_| format!("{player:?}: the team must be 0 or 1"))?;
                if id.trim().is_empty() || team > 1 {
                    return Err(format!("{player:?}: needs an agent id and team 0 or 1"));
                }
                parsed.players.push((id.to_string(), team));
            }
        }
    }
    if parsed.players.is_empty() {
        return Err("name at least one AGENT_ID:TEAM".into());
    }
    Ok(parsed)
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse(std::env::args().skip(1))?;
    let listener = GameListener::bind(args.game_port)?;
    println!(
        "waiting for the game on 127.0.0.1:{}; start it with: {}",
        args.game_port,
        rlbot_args(args.game_port).join(" ")
    );
    let _child = args
        .launch
        .as_deref()
        .map(|exe| GameLaunch::new(exe, args.game_port).spawn())
        .transpose()?;
    let stream = listener
        .wait_for_connection(Duration::from_secs(300))?
        .ok_or("the game did not connect within five minutes")?;
    stream.set_nonblocking(true)?;
    println!("the game connected");

    // Unlimited: the game keeps its own clock; this only fills the configuration.
    let configuration = bot_match(&args.players, MatchLengthMutator::Unlimited);
    let backend = LinkBackend::new(stream, configuration).with_spawning(args.spawn);
    // Pacing is ignored for a game: it decides when a packet exists.
    let mut core = Core::bind(("127.0.0.1", args.port), backend, Pacing::RealTime)?;
    let end = core.run(&AtomicBool::new(false), |event| match event {
        Event::Listening(addr) => println!("serving bots on {addr}"),
        event => println!("{event:?}"),
    })?;
    println!("rb_rl_bridge finished: {end:?}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rb_rl_bridge: {e}");
            ExitCode::FAILURE
        }
    }
}

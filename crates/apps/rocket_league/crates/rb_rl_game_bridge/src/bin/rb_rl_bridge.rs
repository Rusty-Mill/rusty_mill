//! RLBot's middleman job against the real game: Rocket League on one side, RLBot v5 bots on
//! the other.
//!
//! ```text
//! rb_rl_bridge [--game-port N] [--port N] [--launch RocketLeague.exe] [--no-spawn]
//!              [--record-in FILE] [--record-out FILE] AGENT_ID:TEAM...
//! ```
//!
//! Listens for the game on `--game-port` (default 23233) and serves bots on `--port` (default
//! 23234). Start the game with the arguments printed here (or pass `--launch`) and a match
//! loading. Once the map is up the bridge spawns one bot car per `AGENT_ID:TEAM`, in that order,
//! as RLBot does; with `--no-spawn` it expects those cars to exist already. Then start the bots
//! with `RLBOT_SERVER_PORT` set (the port is taken at startup, so bots may also start first and
//! wait for the game). Run live on 2026-10-11 (Epic, Freeplay); see the README.
//! The bridge must be running before the game starts: the game dials it once at launch, and
//! spawning waits for the map announcement that follows. `--record-in` / `--record-out` files
//! contain the players' display names; scrub them before sharing.

use std::fs::File;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rb_rl_game_bridge::{
    rlbot_args, GameLaunch, GameListener, LinkBackend, IDEAL_GAME_PORT, RLBOT_SOCKETS_PORT,
};
use rb_rlbot_core::{bot_match, Core, Event, Pacing};
use rb_rlbot_wire::MatchLengthMutator;

/// A stream that copies what passes through it to files, to see what the game and the bridge
/// actually said to each other (`--record-in`: from the game; `--record-out`: to it).
struct Tee<S> {
    inner: S,
    inbound: Option<File>,
    outbound: Option<File>,
}

/// Copies `bytes` to a recorder. If the file fails, recording stops with one warning: the bytes
/// have already crossed the socket, so the stream must not be told they failed.
fn record(file: &mut Option<File>, bytes: &[u8], name: &str) {
    if let Some(f) = file {
        if let Err(e) = f.write_all(bytes) {
            eprintln!("rb_rl_bridge: recording {name} stopped: {e}");
            *file = None;
        }
    }
}

impl<S: Read> Read for Tee<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        record(&mut self.inbound, &buf[..n], "--record-in");
        Ok(n)
    }
}

impl<S: Write> Write for Tee<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        record(&mut self.outbound, &buf[..n], "--record-out");
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct Args {
    game_port: u16,
    port: u16,
    launch: Option<String>,
    spawn: bool,
    record_in: Option<String>,
    record_out: Option<String>,
    players: Vec<(String, u32)>,
    length: MatchLengthMutator,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut parsed = Args {
        game_port: IDEAL_GAME_PORT,
        port: RLBOT_SOCKETS_PORT,
        launch: None,
        spawn: true,
        record_in: None,
        record_out: None,
        players: Vec::new(),
        length: MatchLengthMutator::FiveMinutes,
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
            "--match-length" => {
                parsed.length = match value("--match-length")?.as_str() {
                    "5" => MatchLengthMutator::FiveMinutes,
                    "10" => MatchLengthMutator::TenMinutes,
                    "20" => MatchLengthMutator::TwentyMinutes,
                    "unlimited" => MatchLengthMutator::Unlimited,
                    _ => return Err("--match-length: expected 5, 10, 20 or unlimited".into()),
                }
            }
            "--record-in" => parsed.record_in = Some(value("--record-in")?),
            "--record-out" => parsed.record_out = Some(value("--record-out")?),
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

fn open_recordings(args: &Args) -> io::Result<(Option<File>, Option<File>)> {
    // Exclusive creation rejects aliases, symlinks and existing files without truncation.
    let inbound = args.record_in.as_ref().map(File::create_new).transpose()?;
    let outbound = args.record_out.as_ref().map(File::create_new).transpose()?;
    Ok((inbound, outbound))
}

/// Takes the bots' port. Done before the game is launched so a clash fails now, with the port
/// named, and not after the game has dialled in and been left with nobody to talk to; bots that
/// start while the game is still loading connect and wait instead of being refused.
fn bind_bots(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port)).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "bots port 127.0.0.1:{port} is not available ({e}); choose another with --port"
            ),
        )
    })
}

/// `line` as the bridge prints it: led by the seconds since the process started, so a log says
/// when each step happened (the game's first message comes a while after it dials).
fn stamped(elapsed: Duration, line: &str) -> String {
    format!("[{:8.3}s] {line}", elapsed.as_secs_f64())
}

/// Prints `line` with its timestamp. The start is the first call, which is the first line.
fn say(line: &str) {
    static START: OnceLock<Instant> = OnceLock::new();
    println!(
        "{}",
        stamped(START.get_or_init(Instant::now).elapsed(), line)
    );
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse(std::env::args().skip(1))?;
    let (inbound, outbound) = open_recordings(&args)?;
    let listener = GameListener::bind(args.game_port)?;
    let bots = bind_bots(args.port)?;
    // Listening is announced here, before the game is found: the game's first message can come a
    // while after it dials, and what starts the bots (the live-test runner) must not wait on it.
    say(&format!("serving bots on {}", bots.local_addr()?));
    say(&format!(
        "waiting for the game on 127.0.0.1:{}; start it with: {}",
        args.game_port,
        rlbot_args(args.game_port).join(" ")
    ));
    let _child = args
        .launch
        .as_deref()
        .map(|exe| GameLaunch::new(exe, args.game_port).spawn())
        .transpose()?;
    // Only a connection that speaks as the game does is taken for it, so a stray local connection
    // cannot hold the game's place. The game's first message can come a while after it dials.
    let stream = listener
        .wait_for_game(Duration::from_secs(300), say)?
        .ok_or("the game did not connect within five minutes")?;
    stream.set_nonblocking(true)?;
    say("the game connected");

    // This declares the attached match; it does not change the game's settings.
    let configuration = bot_match(&args.players, args.length);
    let tee = Tee {
        inner: stream,
        inbound,
        outbound,
    };
    let backend = LinkBackend::new(tee, configuration).with_spawning(args.spawn);
    // Pacing is ignored for a game: it decides when a packet exists.
    let mut core = Core::from_listener(bots, backend, Pacing::RealTime)?;
    let end = core.run(&AtomicBool::new(false), |event| match event {
        // Already announced above, when the port was taken.
        Event::Listening(_) => {}
        event => say(&format!("{event:?}")),
    })?;
    say(&format!("rb_rl_bridge finished: {end:?}"));
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_led_by_its_seconds_in_a_fixed_width_stamp() {
        assert_eq!(stamped(Duration::ZERO, "x"), "[   0.000s] x");
        assert_eq!(stamped(Duration::from_millis(45_678), "x"), "[  45.678s] x");
        // Past a thousand seconds the stamp grows rather than truncating.
        assert_eq!(stamped(Duration::from_secs(12_345), "x"), "[12345.000s] x");
    }

    #[test]
    fn attached_duration_is_declared_not_always_unlimited() {
        let args = parse(["bot:0".to_string()].into_iter()).unwrap();
        assert_eq!(args.length, MatchLengthMutator::FiveMinutes);
        let args = parse(
            ["--match-length", "unlimited", "bot:0"]
                .map(str::to_string)
                .into_iter(),
        )
        .unwrap();
        assert_eq!(args.length, MatchLengthMutator::Unlimited);
    }
    #[test]
    fn a_taken_bots_port_is_refused_by_name_before_anything_is_launched() {
        let held = bind_bots(0).unwrap();
        let port = held.local_addr().unwrap().port();
        let e = bind_bots(port).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AddrInUse);
        assert!(e.to_string().contains(&port.to_string()), "{e}");
        drop(held);
        assert!(bind_bots(port).is_ok(), "free again once released");
    }
    #[test]
    fn exclusive_recording_open_rejects_same_path_without_truncating() {
        let path = std::env::temp_dir().join(format!("rb-record-{}", std::process::id()));
        let mut args = parse(["bot:0".to_string()].into_iter()).unwrap();
        args.record_in = Some(path.to_string_lossy().into_owned());
        args.record_out = args.record_in.clone();
        assert_eq!(
            open_recordings(&args).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        let mut first = File::options().write(true).open(&path).unwrap();
        first.write_all(b"existing bytes").unwrap();
        assert_eq!(
            open_recordings(&args).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"existing bytes");
        std::fs::remove_file(path).unwrap();
    }
}

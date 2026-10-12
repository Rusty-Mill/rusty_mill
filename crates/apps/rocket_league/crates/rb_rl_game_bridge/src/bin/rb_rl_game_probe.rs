//! Listens the way RLBot does, decodes what Rocket League sends, and says what it saw. The way
//! to check the game-socket format against a live game, and to record real traffic as fixtures.
//!
//! ```text
//! rb_rl_game_probe [--port N] [--seconds S] [--record FILE] [--launch RocketLeague.exe]
//! ```
//!
//! Start the game with the arguments this prints (Steam: set them as launch options; or pass
//! `--launch` with the executable's path to have the probe start it). It waits up to two
//! minutes for the game to connect, then prints one line per message until the game closes
//! the socket or `--seconds` pass (default 30). `--record` writes the raw framed bytes, which
//! `rb_rlbot_wire::frame` and `decode_game_packet` read back.

use std::io::{Read, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use rb_rl_game_bridge::{
    decode_game_packet, rlbot_args, GameEvent, GameFolder, GameLaunch, GameListener,
    IDEAL_GAME_PORT,
};
use rb_rlbot_wire::frame::FrameDecoder;

struct Args {
    port: u16,
    seconds: u64,
    record: Option<String>,
    launch: Option<String>,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut parsed = Args {
        port: IDEAL_GAME_PORT,
        seconds: 30,
        record: None,
        launch: None,
    };
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--port" => {
                parsed.port = value("--port")?
                    .parse()
                    .map_err(|_| "--port: not a port".to_string())?;
            }
            "--seconds" => {
                parsed.seconds = value("--seconds")?
                    .parse()
                    .map_err(|_| "--seconds: not a number".to_string())?;
            }
            "--record" => parsed.record = Some(value("--record")?),
            "--launch" => parsed.launch = Some(value("--launch")?),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(parsed)
}

fn kind(event: &GameEvent) -> String {
    match event {
        GameEvent::Other(tag) => format!("other({tag})"),
        e => format!("{e:?}")
            .split([' ', '{', '('])
            .next()
            .unwrap_or_default()
            .to_string(),
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse(std::env::args().skip(1))?;
    let listener = GameListener::bind(args.port)?;
    println!(
        "listening on 127.0.0.1:{}; start the game with: {}",
        listener.local_addr()?.port(),
        rlbot_args(args.port).join(" ")
    );
    let _child = args
        .launch
        .as_deref()
        .map(|exe| GameLaunch::new(exe, args.port).spawn())
        .transpose()?;
    let mut stream = listener
        .wait_for_connection(Duration::from_secs(120))?
        .ok_or("the game did not connect within two minutes")?;
    println!("the game connected");
    let mut record = args
        .record
        .as_ref()
        .map(std::fs::File::create)
        .transpose()?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    let end = Instant::now() + Duration::from_secs(args.seconds);
    let (mut decoder, mut folder) = (FrameDecoder::new(), GameFolder::new());
    let mut chunk = [0u8; 8192];
    let (mut messages, mut undecodable) = (0u32, 0u32);
    while Instant::now() < end {
        let n = match stream.read(&mut chunk) {
            Ok(0) => {
                println!("the game closed the socket");
                break;
            }
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e.into()),
        };
        if let Some(file) = &mut record {
            file.write_all(&chunk[..n])?;
        }
        decoder.push(&chunk[..n]);
        while let Some(payload) = decoder.next_frame() {
            messages += 1;
            match decode_game_packet(&payload) {
                Ok(bundle) => {
                    folder.apply(&bundle);
                    let p = folder.packet();
                    let kinds: Vec<String> = bundle.events.iter().map(kind).collect();
                    println!(
                        "#{messages} +{}f {:?} cars={} frame={} events=[{}]",
                        bundle.delta_physics_frames,
                        p.match_info.match_phase,
                        p.players.len(),
                        p.match_info.frame_num,
                        kinds.join(", ")
                    );
                }
                Err(e) => {
                    undecodable += 1;
                    println!("#{messages} undecodable ({} bytes): {e}", payload.len());
                }
            }
        }
    }
    println!("{messages} messages, {undecodable} undecodable");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rb_rl_game_probe: {e}");
            ExitCode::FAILURE
        }
    }
}

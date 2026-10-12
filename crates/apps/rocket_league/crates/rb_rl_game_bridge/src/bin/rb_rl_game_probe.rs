//! Listens the way RLBot does, decodes what Rocket League sends, and says what it saw. The way
//! to check the game-socket format against a live game, and to record real traffic as fixtures.
//!
//! ```text
//! rb_rl_game_probe [--port N] [--seconds S] [--record FILE] [--launch RocketLeague.exe]
//! rb_rl_game_probe --analyze FILE
//! rb_rl_game_probe --commands FILE
//! ```
//!
//! Start the game with the arguments this prints (Steam: set them as launch options; or pass
//! `--launch` with the executable's path to have the probe start it). It waits up to two
//! minutes for the game to connect, then prints one line per message until the game closes
//! the socket or `--seconds` pass (default 30). `--record` writes the raw framed bytes, which
//! `rb_rlbot_wire::frame` and `decode_game_packet` read back. `--analyze` summarises such a
//! recording (what events and phases it holds, the map, the spawns, the physics ranges) and exits.

use std::collections::BTreeMap;
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
    analyze: Option<String>,
    commands: Option<String>,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut parsed = Args {
        port: IDEAL_GAME_PORT,
        seconds: 30,
        record: None,
        launch: None,
        analyze: None,
        commands: None,
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
            "--analyze" => parsed.analyze = Some(value("--analyze")?),
            "--commands" => parsed.commands = Some(value("--commands")?),
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

/// Prints the commands in a recording of what was sent to the game (`rb_rl_bridge --record-out`).
fn commands(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let mut decoder = FrameDecoder::new();
    decoder.push(&bytes);
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    let mut shown = 0;
    while let Some(payload) = decoder.next_frame() {
        for (id, command) in rb_rl_game_bridge::decode_controller_packet(&payload)? {
            let name = format!("{command:?}");
            let name = name
                .split([' ', '{', '('])
                .next()
                .unwrap_or_default()
                .to_string();
            *counts.entry(name.clone()).or_default() += 1;
            // Print every command that is not a per-tick input, and a few inputs.
            if name != "PlayerInput" || shown < 3 {
                if name == "PlayerInput" {
                    shown += 1;
                }
                println!("command #{id}: {command:?}");
            }
        }
    }
    println!("--- command counts: {counts:?}");
    Ok(())
}

/// What a recording shows of one car: where it started and ended and its top speed (uu/s).
struct CarTrack {
    name: String,
    first: (f32, f32),
    last: (f32, f32),
    top_speed: f32,
}

/// Summarises a recording made with `--record`.
fn analyze(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let mut decoder = FrameDecoder::new();
    decoder.push(&bytes);
    let (mut messages, mut undecodable, mut frames) = (0u32, 0u32, 0u64);
    let mut kinds: BTreeMap<String, u32> = BTreeMap::new();
    let mut first_seen: BTreeMap<String, u32> = BTreeMap::new();
    let mut clock: Vec<i16> = Vec::new();
    let mut cars: BTreeMap<i32, CarTrack> = BTreeMap::new();
    let mut deltas: BTreeMap<u8, u32> = BTreeMap::new();
    let mut phases: Vec<String> = Vec::new();
    let (mut ball_z, mut car_z) = ((f32::MAX, f32::MIN), (f32::MAX, f32::MIN));
    let mut folder = GameFolder::new();
    while let Some(payload) = decoder.next_frame() {
        messages += 1;
        let bundle = match decode_game_packet(&payload) {
            Ok(b) => b,
            Err(e) => {
                undecodable += 1;
                println!(
                    "undecodable message #{messages} ({} bytes): {e}",
                    payload.len()
                );
                continue;
            }
        };
        frames += u64::from(bundle.delta_physics_frames);
        *deltas.entry(bundle.delta_physics_frames).or_default() += 1;
        for event in &bundle.events {
            *kinds.entry(kind(event)).or_default() += 1;
            first_seen.entry(kind(event)).or_insert(messages);
            match event {
                GameEvent::GameSpawn {
                    map_name,
                    mode,
                    team_size,
                    gravity,
                } => println!(
                    "GameSpawn: map {map_name:?}, mode {mode}, team size {team_size}, gravity {gravity:?}"
                ),
                GameEvent::GoalSpawn {
                    id,
                    team,
                    center,
                    direction,
                    width,
                    height,
                } => println!(
                    "GoalSpawn: id {id} team {team} centre ({}, {}, {}) direction ({}, {}, {}) {width} x {height}",
                    center.x, center.y, center.z, direction.x, direction.y, direction.z
                ),
                GameEvent::PlayerSpawn {
                    id,
                    command_id,
                    name,
                    team,
                } => println!(
                    "PlayerSpawn: actor {id} command_id {command_id:?} name {name:?} team {team}"
                ),
                GameEvent::TimeChange { display_seconds } => clock.push(*display_seconds),
                GameEvent::BallSpawn { id } => println!("BallSpawn: actor {id}"),
                GameEvent::StateChange { phase, is_overtime } => {
                    println!("StateChange: {phase:?} overtime {is_overtime}");
                }
                _ => {}
            }
        }
        folder.apply(&bundle);
        let p = folder.packet();
        let phase = format!("{:?}", p.match_info.match_phase);
        if phases.last() != Some(&phase) {
            phases.push(phase);
        }
        for car in &p.players {
            let (l, v) = (car.physics.location, car.physics.velocity);
            let speed = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
            let track = cars.entry(car.player_id).or_insert_with(|| CarTrack {
                name: car.name.clone(),
                first: (l.x, l.y),
                last: (l.x, l.y),
                top_speed: 0.0,
            });
            track.last = (l.x, l.y);
            track.top_speed = track.top_speed.max(speed);
        }
        if let Some(ball) = p.balls.first() {
            let z = ball.physics.location.z;
            ball_z = (ball_z.0.min(z), ball_z.1.max(z));
        }
        if let Some(car) = p.players.first() {
            let z = car.physics.location.z;
            car_z = (car_z.0.min(z), car_z.1.max(z));
        }
    }
    println!("--- {messages} messages, {undecodable} undecodable, {frames} physics frames");
    println!("event counts: {kinds:?}");
    println!("first message (1-based) of each kind: {first_seen:?}");
    println!("delta_physics_frames histogram: {deltas:?}");
    println!("phases in order: {}", phases.join(" -> "));
    let ends = |v: &[i16]| {
        let head: Vec<_> = v.iter().take(4).collect();
        let tail: Vec<_> = v.iter().rev().take(4).rev().collect();
        format!("first {head:?} ... last {tail:?}")
    };
    println!("scoreboard clock: {}", ends(&clock));
    let p = folder.packet();
    println!(
        "last state: {} cars, {} pads ({} inactive), scores {:?}, clock {}",
        p.players.len(),
        p.boost_pads.len(),
        p.boost_pads.iter().filter(|b| !b.is_active).count(),
        p.teams.iter().map(|t| t.score).collect::<Vec<_>>(),
        p.match_info.game_time_remaining
    );
    println!("ball z range {ball_z:?}, car z range {car_z:?}");
    for (index, car) in &cars {
        let moved =
            ((car.last.0 - car.first.0).powi(2) + (car.last.1 - car.first.1).powi(2)).sqrt();
        println!(
            "player {index} {:?}: moved {moved:.0} uu (first ({:.0}, {:.0}) last ({:.0}, {:.0})), top speed {:.0} uu/s",
            car.name, car.first.0, car.first.1, car.last.0, car.last.1, car.top_speed
        );
    }
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse(std::env::args().skip(1))?;
    if let Some(path) = &args.analyze {
        return analyze(path);
    }
    if let Some(path) = &args.commands {
        return commands(path);
    }
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

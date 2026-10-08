//! Unattended batch runner: for each scenario, starts a capture through the
//! plugin's job file, starts a freeplay match with the tape bot through
//! RLBot core's socket, waits for the tape to finish by counting physics
//! frames, stops the capture and the match. Writes `<out>/<scenario>_run<k>.jsonl`.
//!
//! Needs BakkesMod running with the capture plugin (1.4+) set to load at game
//! start, and RLBot core installed. See tools/rb_tape_bot/README.md.
//!
//! Usage: `rb_run_tapes [--out DIR] [--repeat N] [scenario names...]`
//! (no names: every file in `scenarios/`).

use std::{
    error::Error,
    fmt::Write as _,
    fs::{self, File},
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread::sleep,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rb_scenario::Scenario;
use rlbot::{
    flat::{
        ConnectionSettings, CoreMessage, CustomBot, DebugRendering, EnvironmentVariable,
        ExistingMatchBehavior, FieldInfo, GameMode, GamePacket, InitComplete, Launcher,
        MatchConfiguration, MatchLengthMutator, MutatorSettings, PlayerClass, PlayerConfiguration,
        StopCommand,
    },
    RLBotConnection,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const CORE_ADDR: &str = "127.0.0.1:23234";
/// Distance (uu) from the scenario's start location that counts as "the bot
/// has set the start state". Loose on purpose: a fast start (the nose landing
/// falls at 1600 uu/s) is at the exact start for one tick only and 50 uu away
/// the next, and the first ticks after a state set can be missing (O009).
/// Kickoff spawns are all more than 800 uu from every start except
/// `speed_flip`'s, which is a spawn point; that one is detected a few frames
/// early, which `TAIL_FRAMES` absorbs.
const START_TOLERANCE_UU: f32 = 300.0;
/// Frames to keep recording after the tape's last tick.
const TAIL_FRAMES: u32 = 180;
/// Latest physics frame of a new match at which the start state can be set.
const MAX_START_FRAME: u32 = 300;
const HEARTBEAT_FRESH: Duration = Duration::from_secs(5);
const GAME_LAUNCH_TIMEOUT: Duration = Duration::from_secs(300);
const MATCH_START_TIMEOUT: Duration = Duration::from_secs(120);
const JOB_ACK_TIMEOUT: Duration = Duration::from_secs(15);

struct Args {
    out: PathBuf,
    repeat: u32,
    names: Vec<String>,
}

/// The bot package directory (`tools/rb_tape_bot`), fixed at compile time.
fn package_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> Result<PathBuf> {
    Ok(package_dir()
        .parent()
        .and_then(Path::parent)
        .ok_or("package is not two levels below the repository root")?
        .to_path_buf())
}

fn env_dir(var: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os(var).ok_or(format!("{var} is not set"))?,
    ))
}

/// `<BakkesMod data>/rusty_bullet_capture`, where the plugin looks for jobs.
fn job_dir() -> Result<PathBuf> {
    Ok(env_dir("APPDATA")?
        .join("bakkesmod")
        .join("bakkesmod")
        .join("data")
        .join("rusty_bullet_capture"))
}

/// UTC `YYYYMMDD-HHMMSS` from the system clock (Hinnant's civil-from-days).
fn timestamp() -> Result<String> {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Ok(format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    ))
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        out: repo_root()?
            .join("replays")
            .join(format!("batch_{}", timestamp()?)),
        repeat: 1,
        names: Vec::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--out" => args.out = it.next().ok_or("--out needs a directory")?.into(),
            "--repeat" => {
                args.repeat = it.next().ok_or("--repeat needs a number")?.parse()?;
            }
            name => args.names.push(name.to_owned()),
        }
    }
    Ok(args)
}

/// Scenario (name, path) pairs, sorted; all of `scenarios/` unless named
/// (a name may also be a file in `experiments/`).
fn list_scenarios(names: &[String]) -> Result<Vec<(String, PathBuf)>> {
    let dir = package_dir().join("scenarios");
    let mut found = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            found.push((stem.to_owned(), path));
        }
    }
    found.sort();
    if names.is_empty() {
        return Ok(found);
    }
    let experiments = package_dir().join("experiments");
    names
        .iter()
        .map(|name| {
            if let Some(found) = found.iter().find(|(n, _)| n == name) {
                return Ok(found.clone());
            }
            // Not shipped: an experiment variant, which stays out of the
            // default set.
            let path = experiments.join(format!("{name}.json"));
            if path.is_file() {
                return Ok((name.clone(), path));
            }
            Err(format!(
                "no scenario named '{name}' in {} or {}",
                dir.display(),
                experiments.display()
            )
            .into())
        })
        .collect()
}

/// A fresh connection that also receives game packets.
fn connect() -> Result<RLBotConnection> {
    let mut conn = RLBotConnection::new(CORE_ADDR)?;
    conn.send_packet(ConnectionSettings {
        wants_ball_predictions: false,
        wants_comms: false,
        close_between_matches: false,
        agent_id: String::new(),
    })?;
    // Core only distributes game packets to a session after InitComplete.
    conn.send_packet(InitComplete {})?;
    conn.set_nonblocking(true)?;
    Ok(conn)
}

/// Connects to core, starting `RLBotServer.exe` (stdout to `core.log`) when
/// nothing listens yet.
fn ensure_core(out: &Path) -> Result<()> {
    if RLBotConnection::new(CORE_ADDR).is_ok() {
        return Ok(());
    }
    let exe = env_dir("LOCALAPPDATA")?
        .join("RLBot5")
        .join("bin")
        .join("RLBotServer.exe");
    println!("core is not running; starting {}", exe.display());
    let log = File::create(out.join("core.log"))?;
    Command::new(&exe)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", exe.display()))?;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if RLBotConnection::new(CORE_ADDR).is_ok() {
            return Ok(());
        }
        sleep(Duration::from_secs(1));
    }
    Err("core did not accept connections within 60 s".into())
}

/// The match for a scenario: one tape bot for a one-car scenario; one hivemind
/// process driving every car of team 0 when there are several and all share a
/// team (they share one clock); and, when a car is on the other team, one
/// player per car (the first plays the tape, the rest stay neutral).
fn match_configuration(
    name: &str,
    scenario_path: &Path,
    teams: &[u32],
) -> Result<MatchConfiguration> {
    let tape = scenario_path
        .to_str()
        .ok_or("scenario path is not UTF-8")?
        .replace('\\', "/");
    let root_dir = package_dir()
        .to_str()
        .ok_or("package path is not UTF-8")?
        .to_owned();
    let hive = teams.len() > 1 && teams.iter().all(|team| *team == teams[0]);
    let player = |index: usize, team: u32| PlayerConfiguration {
        variety: PlayerClass::CustomBot(Box::new(CustomBot {
            name: format!("RB Tape: {name}"),
            root_dir: root_dir.clone(),
            // cmd.exe needs backslashes here (see bots/*.bot.toml).
            run_command: if hive {
                r"target\release\rb_tape_hive.exe".into()
            } else {
                r"target\release\rb_tape_bot.exe".into()
            },
            agent_id: if hive {
                "rusty_bullet/tape_hive".into()
            } else {
                format!("rusty_bullet/tape_bot_{index}")
            },
            hivemind: hive,
            environment: Some(vec![EnvironmentVariable {
                name: "RB_TAPE".into(),
                value: tape.clone(),
            }]),
            ..Default::default()
        })),
        team,
        player_id: 0,
    };
    let players = if teams.is_empty() {
        vec![player(0, 0)]
    } else {
        teams
            .iter()
            .enumerate()
            .map(|(index, team)| player(index, *team))
            .collect()
    };
    Ok(MatchConfiguration {
        launcher: Launcher::Epic,
        auto_start_agents: true,
        wait_for_agents: true,
        game_map_upk: "Stadium_P".into(),
        player_configurations: players,
        game_mode: GameMode::Soccar,
        // Core crashes on a missing mutator table.
        mutators: Some(Box::new(MutatorSettings {
            match_length: MatchLengthMutator::Unlimited,
            ..Default::default()
        })),
        existing_match_behavior: ExistingMatchBehavior::Restart,
        enable_rendering: DebugRendering::OffByDefault,
        enable_state_setting: true,
        freeplay: true,
        ..Default::default()
    })
}

fn heartbeat_fresh(dir: &Path) -> bool {
    fs::metadata(dir.join("heartbeat.txt"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < HEARTBEAT_FRESH)
}

fn capturing(dir: &Path) -> bool {
    fs::read_to_string(dir.join("heartbeat.txt")).is_ok_and(|t| t.contains("capturing=1"))
}

/// Writes a job and waits for the plugin to consume (delete) it.
fn send_job(dir: &Path, json: &str) -> Result<()> {
    let tmp = dir.join("job.tmp");
    fs::write(&tmp, json)?;
    // Rename so the plugin never reads a half-written file.
    fs::rename(&tmp, dir.join("job.json"))?;
    let deadline = Instant::now() + JOB_ACK_TIMEOUT;
    while dir.join("job.json").exists() {
        if Instant::now() > deadline {
            return Err(format!("plugin did not take the job {json} within 15 s").into());
        }
        sleep(Duration::from_millis(250));
    }
    Ok(())
}

fn json_escape(path: &Path) -> Result<String> {
    let text = path.to_str().ok_or("capture path is not UTF-8")?;
    Ok(text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Reads packets until `done` returns true or `timeout` passes.
fn pump(
    conn: &mut RLBotConnection,
    timeout: Duration,
    what: &str,
    mut observe: impl FnMut(&CoreMessage),
    mut done: impl FnMut(&GamePacket) -> bool,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match conn.recv_packet() {
            Ok(message) => {
                observe(&message);
                if let CoreMessage::GamePacket(packet) = &message {
                    if done(packet) {
                        return Ok(());
                    }
                }
            }
            Err(rlbot::RLBotError::Connection(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                sleep(Duration::from_millis(2));
            }
            Err(e) => return Err(format!("core connection failed while {what}: {e}").into()),
        }
    }
    Err(format!("timed out after {timeout:?} {what}").into())
}

/// Boost pad sidecar of a run (`<capture>.pads.jsonl`): one `field` line with
/// every pad's position and size, then one `pads` line for the first logged
/// packet (all pads) and one for each later packet in which any pad's
/// `is_active` flipped, with the first car's position and boost at that
/// frame. Pad order is the game's (by y, then x). The boost value alone
/// cannot say when a pad was taken; this can (PARITY-PLAN workstream C).
struct PadLog {
    out: File,
    /// The last `field` line written; a different `FieldInfo` writes a new one.
    last_field: Option<String>,
    last_active: Option<Vec<bool>>,
}

impl PadLog {
    fn create(path: &Path) -> Result<Self> {
        Ok(Self {
            out: File::create(path)?,
            last_field: None,
            last_active: None,
        })
    }

    /// Writes the field description when it differs from the last one written.
    /// Core can send a stale one (the previous match's map, 38 pads instead of
    /// 34, seen on the first run after the GUI's match), so the reader uses
    /// the last `field` line before the first `pads` line.
    fn field(&mut self, info: &FieldInfo) -> Result<()> {
        // An empty list is core's answer before any match exists.
        if info.boost_pads.is_empty() {
            return Ok(());
        }
        let line = field_line(info);
        if self.last_field.as_deref() == Some(line.as_str()) {
            return Ok(());
        }
        writeln!(self.out, "{line}")?;
        self.last_field = Some(line);
        Ok(())
    }

    /// Logs `packet` when it is the first, or when a pad changed state.
    fn packet(&mut self, packet: &GamePacket) -> Result<()> {
        let active: Vec<bool> = packet.boost_pads.iter().map(|pad| pad.is_active).collect();
        let changed: Vec<usize> = match &self.last_active {
            None => (0..active.len()).collect(),
            Some(last) => (0..active.len())
                .filter(|&i| last.get(i) != Some(&active[i]))
                .collect(),
        };
        self.last_active = Some(active);
        if changed.is_empty() {
            return Ok(());
        }
        writeln!(self.out, "{}", pads_line(packet, &changed))?;
        Ok(())
    }
}

fn field_line(info: &FieldInfo) -> String {
    let mut line = String::from(r#"{"field":["#);
    for (i, pad) in info.boost_pads.iter().enumerate() {
        if i > 0 {
            line.push(',');
        }
        let l = &pad.location;
        // Writing to a String cannot fail.
        let _ = write!(
            line,
            r#"{{"x":{},"y":{},"z":{},"big":{}}}"#,
            l.x, l.y, l.z, pad.is_full_boost
        );
    }
    line.push_str("]}");
    line
}

fn pads_line(packet: &GamePacket, changed: &[usize]) -> String {
    let car = packet.players.first().map_or(String::from("null"), |p| {
        let l = &p.physics.location;
        format!("[{},{},{},{}]", l.x, l.y, l.z, p.boost)
    });
    let mut line = format!(
        r#"{{"frame":{},"car":{},"pads":["#,
        packet.match_info.frame_num, car
    );
    for (n, &i) in changed.iter().enumerate() {
        if n > 0 {
            line.push(',');
        }
        let pad = &packet.boost_pads[i];
        let _ = write!(line, r#"[{},{},{}]"#, i, pad.is_active, pad.timer);
    }
    line.push_str("]}");
    line
}

/// Starts the game with the scenario's match when no plugin heartbeat shows
/// (the plugin loads with the game, so a capture can only be requested once
/// the game is up).
fn ensure_game_up(dir: &Path, config: &MatchConfiguration) -> Result<()> {
    if heartbeat_fresh(dir) {
        return Ok(());
    }
    println!("no plugin heartbeat; launching the game through core");
    let mut conn = connect()?;
    conn.send_packet(config.clone())?;
    // Keep reading while waiting: core drops a session whose outbound
    // queue fills, and a game launch takes tens of seconds.
    pump(
        &mut conn,
        GAME_LAUNCH_TIMEOUT,
        "waiting for the plugin heartbeat (is BakkesMod running and the plugin set to load at game start?)",
        |_| {},
        |_| heartbeat_fresh(dir),
    )?;
    conn.send_packet(StopCommand {
        shutdown_server: false,
    })?;
    // Let the stop land before the real run restarts the match.
    sleep(Duration::from_secs(3));
    Ok(())
}

/// `<capture>.pads.jsonl` next to `<capture>` (`x_run1.jsonl` -> `x_run1.pads.jsonl`).
fn pad_log_path(capture: &Path) -> PathBuf {
    capture.with_extension("pads.jsonl")
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

/// One capture of one scenario into `capture`.
fn run_one(dir: &Path, name: &str, path: &Path, capture: &Path) -> Result<()> {
    let result = run_one_inner(dir, name, path, capture);
    if result.is_err() {
        // Do not leave a capture recording or a match running after a failure.
        if let Err(e) = abort_run(dir) {
            eprintln!("  cleanup after the failure also failed: {e}");
        }
    }
    result
}

fn abort_run(dir: &Path) -> Result<()> {
    send_job(dir, r#"{"stop": true}"#)?;
    connect()?.send_packet(StopCommand {
        shutdown_server: false,
    })?;
    Ok(())
}

fn run_one_inner(dir: &Path, name: &str, path: &Path, capture: &Path) -> Result<()> {
    let scenario = Scenario::from_json(&fs::read_to_string(path)?)?;
    let start = scenario
        .car
        .location
        .ok_or_else(|| format!("{name} has no car.location to detect the start by"))?;
    let config = match_configuration(name, path, &scenario.teams())?;
    ensure_game_up(dir, &config)?;

    send_job(dir, &format!(r#"{{"start": "{}"}}"#, json_escape(capture)?))?;
    sleep(Duration::from_millis(1_200));
    if !capturing(dir) {
        return Err(format!("plugin could not open {} for writing", capture.display()).into());
    }

    let mut pads = PadLog::create(&pad_log_path(capture))?;
    let mut conn = connect()?;
    conn.send_packet(config)?;
    let mut first_frame = 0u32;
    pump(
        &mut conn,
        MATCH_START_TIMEOUT,
        "waiting for the bot to set the start state",
        |m| {
            if let CoreMessage::FieldInfo(info) = m {
                if let Err(e) = pads.field(info) {
                    eprintln!("  pad log: {e}");
                }
            }
        },
        |p| {
            // The bot sets the state within the first few frames of a match; a
            // later frame is the previous match's last packets (pogo ends near
            // prompt_dodge's start).
            let near = p.match_info.frame_num < MAX_START_FRAME
                && p.players.first().is_some_and(|car| {
                    let l = &car.physics.location;
                    distance([l.x, l.y, l.z], start) < START_TOLERANCE_UU
                });
            if near {
                first_frame = p.match_info.frame_num;
            }
            near
        },
    )?;
    let end_frame = first_frame + u32::try_from(scenario.total_ticks())? + 1 + TAIL_FRAMES;
    println!("  start state seen at frame {first_frame}; recording to frame {end_frame}");
    let tape_secs = u64::from(end_frame - first_frame) / 60 + 30;
    pump(
        &mut conn,
        Duration::from_secs(tape_secs),
        "waiting for the tape to end",
        |m| match m {
            CoreMessage::FieldInfo(info) => {
                if let Err(e) = pads.field(info) {
                    eprintln!("  pad log: {e}");
                }
            }
            CoreMessage::GamePacket(p) if p.match_info.frame_num >= first_frame => {
                if let Err(e) = pads.packet(p) {
                    eprintln!("  pad log: {e}");
                }
            }
            _ => {}
        },
        |p| p.match_info.frame_num >= end_frame,
    )?;

    send_job(dir, r#"{"stop": true}"#)?;
    conn.send_packet(StopCommand {
        shutdown_server: false,
    })?;
    Ok(())
}

fn run() -> Result<()> {
    let args = parse_args()?;
    let scenarios = list_scenarios(&args.names)?;
    fs::create_dir_all(&args.out)?;
    let dir = job_dir()?;
    fs::create_dir_all(&dir)?;
    ensure_core(&args.out)?;
    println!(
        "{} scenario(s) x {} repeat(s) -> {}",
        scenarios.len(),
        args.repeat,
        args.out.display()
    );
    for repeat in 1..=args.repeat {
        for (name, path) in &scenarios {
            let capture = args.out.join(format!("{name}_run{repeat}.jsonl"));
            println!("{name} (run {repeat})");
            run_one(&dir, name, path, &capture)
                .map_err(|e| format!("{name} run {repeat} failed: {e}"))?;
            if !capture.exists() {
                return Err(
                    format!("{name} run {repeat}: no capture at {}", capture.display()).into(),
                );
            }
        }
    }
    println!("done: {}", args.out.display());
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("rb_run_tapes: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_is_euclidean() {
        assert!((distance([0.0, 0.0, 0.0], [3.0, 4.0, 0.0]) - 5.0).abs() < 1e-6);
    }

    fn packet_with_pads(frame: u32, active: &[bool]) -> GamePacket {
        let mut packet = GamePacket::default();
        packet.match_info.frame_num = frame;
        packet.boost_pads = active
            .iter()
            .map(|&is_active| rlbot::flat::BoostPadState {
                is_active,
                timer: if is_active { 0.0 } else { 1.5 },
            })
            .collect();
        packet
    }

    fn read_lines(path: &Path) -> Vec<String> {
        fs::read_to_string(path)
            .expect("read pad log")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn pad_log_path_sits_next_to_the_capture() {
        let path = pad_log_path(Path::new("out/fuzz_501_run2.jsonl"));
        assert!(path.ends_with("fuzz_501_run2.pads.jsonl"), "{path:?}");
    }

    #[test]
    fn pad_log_writes_the_first_packet_then_only_changes() {
        let path = std::env::temp_dir().join(format!("rb_pad_log_{}.jsonl", std::process::id()));
        let mut log = PadLog::create(&path).expect("create");
        log.packet(&packet_with_pads(10, &[true, true, true]))
            .expect("first");
        log.packet(&packet_with_pads(11, &[true, true, true]))
            .expect("same");
        log.packet(&packet_with_pads(12, &[true, false, true]))
            .expect("taken");
        log.packet(&packet_with_pads(13, &[true, false, true]))
            .expect("same");
        let lines = read_lines(&path);
        let _ = fs::remove_file(&path);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].contains(r#""frame":10"#) && lines[0].contains("[2,true,0]"),
            "{}",
            lines[0]
        );
        assert!(
            lines[1].contains(r#""frame":12"#) && lines[1].contains("[1,false,1.5]"),
            "{}",
            lines[1]
        );
        assert!(
            !lines[1].contains("[0,"),
            "unchanged pad logged: {}",
            lines[1]
        );
    }

    #[test]
    fn pad_log_skips_an_empty_field_and_rewrites_only_a_changed_one() {
        let path = std::env::temp_dir().join(format!("rb_pad_field_{}.jsonl", std::process::id()));
        let mut log = PadLog::create(&path).expect("create");
        log.field(&FieldInfo::default()).expect("empty");
        let mut info = FieldInfo::default();
        info.boost_pads.push(rlbot::flat::BoostPad {
            location: rlbot::flat::Vector3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            is_full_boost: true,
        });
        log.field(&info).expect("real");
        log.field(&info).expect("again");
        info.boost_pads[0].location.x = 5.0;
        log.field(&info).expect("changed");
        let lines = read_lines(&path);
        let _ = fs::remove_file(&path);
        assert_eq!(
            lines,
            [
                r#"{"field":[{"x":1,"y":2,"z":3,"big":true}]}"#,
                r#"{"field":[{"x":5,"y":2,"z":3,"big":true}]}"#,
            ]
        );
    }

    #[test]
    fn json_escape_doubles_backslashes_and_quotes() {
        let escaped = json_escape(Path::new(r#"C:\a\"b"#)).expect("utf-8");
        assert_eq!(escaped, r#"C:\\a\\\"b"#);
    }

    #[test]
    fn every_shipped_scenario_is_listed_and_unknown_names_fail() {
        let all = list_scenarios(&[]).expect("list");
        assert!(all.len() >= 11, "found {}", all.len());
        assert!(list_scenarios(&["no_such_scenario".into()]).is_err());
        let one = list_scenarios(&["prompt_dodge".into()]).expect("named");
        assert_eq!(one.len(), 1);
    }

    #[test]
    fn match_configuration_is_freeplay_with_state_setting_and_the_tape() {
        let (name, path) = list_scenarios(&["pogo".into()]).expect("list").remove(0);
        let config = match_configuration(&name, &path, &[0]).expect("config");
        assert!(config.freeplay && config.enable_state_setting);
        let PlayerClass::CustomBot(bot) = &config.player_configurations[0].variety else {
            panic!("not a custom bot");
        };
        let env = bot.environment.as_ref().expect("env");
        assert_eq!(env[0].name, "RB_TAPE");
        assert!(
            env[0].value.ends_with("scenarios/pogo.json"),
            "{}",
            env[0].value
        );
    }

    #[test]
    fn a_two_car_scenario_is_one_hivemind_team_of_two_players() {
        let (name, path) = list_scenarios(&["pogo".into()]).expect("list").remove(0);
        let config = match_configuration(&name, &path, &[0, 0]).expect("config");
        assert_eq!(config.player_configurations.len(), 2);
        for player in &config.player_configurations {
            let PlayerClass::CustomBot(bot) = &player.variety else {
                panic!("not a custom bot");
            };
            assert!(bot.hivemind, "both cars are one hivemind");
            assert!(bot.run_command.ends_with("rb_tape_hive.exe"));
            assert_eq!(player.team, 0);
        }
        let single = match_configuration(&name, &path, &[0]).expect("config");
        assert_eq!(single.player_configurations.len(), 1);
    }

    #[test]
    fn cars_on_both_teams_are_separate_players_not_a_hivemind() {
        let (name, path) = list_scenarios(&["pogo".into()]).expect("list").remove(0);
        let config = match_configuration(&name, &path, &[0, 1]).expect("config");
        assert_eq!(config.player_configurations.len(), 2);
        let teams: Vec<u32> = config
            .player_configurations
            .iter()
            .map(|p| p.team)
            .collect();
        assert_eq!(teams, vec![0, 1]);
        let mut agent_ids = Vec::new();
        for player in &config.player_configurations {
            let PlayerClass::CustomBot(bot) = &player.variety else {
                panic!("not a custom bot");
            };
            assert!(!bot.hivemind);
            assert!(bot.run_command.ends_with("rb_tape_bot.exe"));
            agent_ids.push(bot.agent_id.clone());
        }
        assert_ne!(agent_ids[0], agent_ids[1], "core pairs players by agent id");
    }

    #[test]
    fn timestamp_has_the_expected_shape() {
        let t = timestamp().expect("clock");
        assert_eq!(t.len(), 15);
        assert_eq!(t.as_bytes()[8], b'-');
    }
}

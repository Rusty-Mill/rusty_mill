//! `rb-verify`: verification pipeline entry point.
//!
//! Thin CLI over `rb_verify_cli`'s scoring functions — argument parsing and
//! human-readable output only, no logic of its own (see `lib.rs` for why
//! the actual wiring lives there instead). Three modes:
//! - `rb-verify <replay-file> <capture-file> [max-timestamp-delta-secs]`:
//!   the mechanical `PHASE-0-EXIT` pipeline check
//!   (`score_replay_against_capture`) — two unrelated recordings, no
//!   physical reason to resemble each other.
//! - `rb-verify --self <capture-file> [max-timestamp-delta-secs]`: the real
//!   fidelity check (`score_capture_against_candidate`,
//!   `RB-PHYSICS-001-FR-077`) — scores a capture's own recorded outcome
//!   against a candidate `rb_physics_bullet` actually simulated from that
//!   same capture's recorded input.
//! - `rb-verify --self-growth <capture-file> [window-secs]
//!   [max-timestamp-delta-secs]`: the divergence-growth diagnostic
//!   (`score_capture_growth`, `RB-VERIFY-003-FR-004`) — the same
//!   candidate-vs-capture comparison as `--self`, but reported per
//!   `window-secs`-wide time window instead of one whole-run number, so
//!   whether the divergence grows gradually or abruptly can be read
//!   directly from the output.
//! - `rb-verify --self-onestep <capture-file> [count]`: the `count`
//!   (default 20) frames with the largest one-step car velocity error
//!   (`one_step_capture`, `RB-VERIFY-003-FR-006`) — each frame predicted
//!   from the recorded frame before it, so the list points at the model's
//!   worst single steps rather than at accumulated divergence.
//! - `rb-verify --self-trace <capture-file> <from-secs> <to-secs>`: the
//!   per-frame trace (`trace_capture`, `RB-VERIFY-003-FR-005`) — every car
//!   at every frame in that window, with the recorded input and the
//!   recorded vs. simulated state side by side, on the same time axis
//!   `--self-growth` prints.
//! - `rb-verify --scenario <scenario.json> [every]`: what the port does with
//!   a scripted scenario (`simulate_scenario`, `docs/research/BOT-CAPTURE-PLAN.md`):
//!   the car's position, velocity and spin every `every` ticks (default 12)
//!   and on every tick a jump is pressed.
//! - `rb-verify --scenario <scenario.json> --against <capture> [--recorded-inputs] [every]`:
//!   the same scenario's recording lined up with that prediction
//!   (`compare_scenario`): per-tick position, velocity and spin error, when
//!   the error first passes 10 and 100 uu, and whether the recorded inputs
//!   played the tape.
//! - `rb-verify --self-kstep <capture-file> [k] [count]`: every frame
//!   predicted `k` ticks ahead (default 30) from the recorded frame `k`
//!   before it, scored like `--self` (`k_step_score`,
//!   `RB-VERIFY-003-FR-008`), then the `count` (default 0) frames with the
//!   largest car velocity error (`k_step_capture`).
//! - `rb-verify --scenario-from <capture> <from-secs> <to-secs> [name]`: a
//!   scenario cut from that window of the capture (`scenario_from_capture`,
//!   `RB-VERIFY-003-FR-012`), written to stdout as JSON, so a human
//!   performance can be replayed by the tape bot.
//! - `rb-verify --seed-first-frame <mode> ...`: a global flag, first on the
//!   command line, that makes every `--self*` mode seed its simulation from
//!   the capture's first frame instead of its first grounded, neutral frame
//!   (`SeedFrame::First`, `RB-VERIFY-003-FR-013`), for recordings whose
//!   start was set by the tape bot.

use rb_capture_ingest::CaptureFileSource;
use rb_domain::divergence::DivergenceScore;
use rb_domain::PhysicsStateSource;
use rb_domain::{ControllerInput, Vec3};
use rb_scenario::Scenario;
use rb_verify_cli::{
    car_frame_spin, compare_scenario, compare_scenario_recorded, default_grid, k_step_capture,
    k_step_score, one_step_capture, rotation_rate, scenario_from_capture,
    score_capture_against_candidate, score_capture_growth, score_replay_against_capture,
    simulate_scenario, sweep, trace_capture, Candidate, SeedFrame, TraceRow, Window,
    DEFAULT_GROWTH_WINDOW_SECS, DEFAULT_K_STEP, DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
};
use std::env;
use std::path::Path;
use std::process::ExitCode;

fn print_score(score: &DivergenceScore) {
    println!("frames compared:    {}", score.frames_compared);
    println!("mean ball distance: {:.2} uu", score.mean_ball_distance);
    println!("max ball distance:  {:.2} uu", score.max_ball_distance);
    println!("car pairs compared: {}", score.cars.pairs_compared);
    println!(
        "mean car position/rotation/velocity distance: {:.2} uu / {:.2} rad / {:.2} uu/s",
        score.cars.mean_position_distance,
        score.cars.mean_rotation_distance,
        score.cars.mean_velocity_distance
    );
    println!(
        "max  car position/rotation/velocity distance: {:.2} uu / {:.2} rad / {:.2} uu/s",
        score.cars.max_position_distance,
        score.cars.max_rotation_distance,
        score.cars.max_velocity_distance
    );
}

fn print_growth(windows: &[(f32, DivergenceScore)]) {
    for (start, score) in windows {
        println!(
            "t={start:>7.2}s  frames={frames:>4}  ball mean/max={mean_ball:>8.2}/{max_ball:>8.2} uu  car mean pos/rot/vel={mean_pos:>8.2} uu / {mean_rot:.2} rad / {mean_vel:>8.2} uu/s",
            frames = score.frames_compared,
            mean_ball = score.mean_ball_distance,
            max_ball = score.max_ball_distance,
            mean_pos = score.cars.mean_position_distance,
            mean_rot = score.cars.mean_rotation_distance,
            mean_vel = score.cars.mean_velocity_distance,
        );
    }
}

fn fmt_vec(v: &Vec3) -> String {
    format!("({:>8.1},{:>8.1},{:>7.1})", v.x, v.y, v.z)
}

/// Angular velocity (rad/s), world frame.
fn fmt_spin(v: &Vec3) -> String {
    format!("({:>5.2},{:>5.2},{:>5.2})", v.x, v.y, v.z)
}

/// Compact input: throttle, steer, pitch/yaw/roll (`-` when unrecovered),
/// then J/B/H for jump/boost/handbrake held (`.` when not).
fn fmt_input(input: Option<ControllerInput>) -> String {
    let Some(i) = input else {
        return "no input".to_string();
    };
    let axis = |a: Option<f32>| a.map_or_else(|| "    -".to_string(), |v| format!("{v:>5.2}"));
    format!(
        "thr {:>5.2} str {:>5.2} p {} y {} r {} {}{}{}",
        i.throttle,
        i.steer,
        axis(i.pitch),
        axis(i.yaw),
        axis(i.roll),
        if i.jump { 'J' } else { '.' },
        if i.boost { 'B' } else { '.' },
        if i.handbrake { 'H' } else { '.' },
    )
}

/// Each row prints both cars' spin in world axes and in the car's own
/// `(forward, side, up)` frame (`car_frame_spin`), and, from the previous
/// row of the same car, the spin
/// implied by the change in recorded and simulated orientation
/// (`rotation_rate`): matching `spin` means a stream's orientations agree
/// with its own angular velocity.
fn print_trace(rows: &[TraceRow]) {
    let mut previous: Vec<&TraceRow> = Vec::new();
    for row in rows {
        let id = row.recorded.player_id;
        let prior = previous.iter().position(|p| p.recorded.player_id == id);
        let rates = prior.map(|i| {
            let p = previous[i];
            let dt = row.t_secs - p.t_secs;
            (
                rotation_rate(&p.recorded.rotation, &row.recorded.rotation, dt),
                rotation_rate(&p.candidate.rotation, &row.candidate.rotation, dt),
            )
        });
        let q_rate = rates.map_or_else(
            || "-".to_string(),
            |(r, c)| format!("rec {} sim {}", fmt_spin(&r), fmt_spin(&c)),
        );
        match prior {
            Some(i) => previous[i] = row,
            None => previous.push(row),
        }
        println!(
            "t={t:>7.3}s car={id} | {input} | rec pos {rp} vel {rv} | sim pos {cp} vel {cv} | err pos {ep:>7.1} vel {ev:>7.1} rot {er:.2} | spin rec {rs} sim {cs} err {es:.2} | car-frame spin rec {ls} sim {lc} | q-rate {q_rate}",
            t = row.t_secs,
            input = fmt_input(row.input),
            rp = fmt_vec(&row.recorded.position),
            rv = fmt_vec(&row.recorded.velocity),
            cp = fmt_vec(&row.candidate.position),
            cv = fmt_vec(&row.candidate.velocity),
            ep = row.position_error(),
            ev = row.velocity_error(),
            er = row.rotation_error(),
            rs = fmt_spin(&row.recorded.angular_velocity),
            cs = fmt_spin(&row.candidate.angular_velocity),
            es = row.spin_error(),
            ls = fmt_spin(&car_frame_spin(&row.recorded.rotation, &row.recorded.angular_velocity)),
            lc = fmt_spin(&car_frame_spin(&row.candidate.rotation, &row.candidate.angular_velocity)),
        );
    }
}

/// The `count` rows with the largest velocity error, worst first: time,
/// input, velocity and spin errors, and the recorded vs. predicted
/// velocity and spin.
fn print_worst_steps(rows: &[TraceRow], count: usize) {
    let mut worst: Vec<&TraceRow> = rows.iter().collect();
    worst.sort_by(|a, b| b.velocity_error().total_cmp(&a.velocity_error()));
    for row in worst.into_iter().take(count) {
        println!(
            "t={t:>7.3}s car={id} | {input} | err vel {ev:>6.1} spin {es:.2} ball {eb:>6.1} | vel rec {rv} sim {cv} | spin rec {rs} sim {cs} | ball vel rec {rb} sim {cb}",
            t = row.t_secs,
            id = row.recorded.player_id,
            input = fmt_input(row.input),
            ev = row.velocity_error(),
            es = row.spin_error(),
            eb = row.ball_velocity_error(),
            rv = fmt_vec(&row.recorded.velocity),
            cv = fmt_vec(&row.candidate.velocity),
            rs = fmt_spin(&row.recorded.angular_velocity),
            cs = fmt_spin(&row.candidate.angular_velocity),
            rb = fmt_vec(&row.recorded_ball.velocity),
            cb = fmt_vec(&row.candidate_ball.velocity),
        );
    }
}

/// Runs a scenario through the port and prints the car's trajectory.
fn run_scenario(path: &str, every: usize) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
    let scenario = Scenario::from_json(&text).map_err(|e| format!("{path}: {e}"))?;
    println!("{}", scenario.name);
    println!("tick  car position (x, y, z)        velocity (x, y, z)             spin (x, y, z)         input");
    let frames = simulate_scenario(&scenario);
    for (tick, frame) in frames.iter().enumerate() {
        let Some(car) = frame.cars.first() else {
            continue;
        };
        let jump = car.input.is_some_and(|input| input.jump);
        if tick % every.max(1) != 0 && !jump {
            continue;
        }
        println!(
            "{tick:>4}  {pos}  {vel}  {spin}  {input}",
            pos = fmt_vec(&car.position),
            vel = fmt_vec(&car.velocity),
            spin = fmt_spin(&car.angular_velocity),
            input = fmt_input(car.input),
        );
    }
    Ok(())
}

/// Prints a scenario's recording against the port's prediction.
fn run_scenario_against(
    path: &str,
    capture: &str,
    every: usize,
    recorded_inputs: bool,
) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
    let scenario = Scenario::from_json(&text).map_err(|e| format!("{path}: {e}"))?;
    let frames = CaptureFileSource::new(capture)
        .frames()
        .map_err(|e| format!("{capture}: {e}"))?;
    let comparison = if recorded_inputs {
        compare_scenario_recorded(&scenario, &frames)
    } else {
        compare_scenario(&scenario, &frames)
    }
    .map_err(|e| format!("{capture}: {e}"))?;
    println!("{}", scenario.name);
    println!(
        "recording starts at capture frame {} (lag {} ticks); {} ticks compared",
        comparison.start_index,
        comparison.lag_ticks,
        comparison.rows.len()
    );
    if let Some(offset) = comparison.input_offset {
        println!("port fed the recorded input (offset {offset}), not the tape");
    }
    println!("tick  pos err  vel err  spin err   recorded position (x, y, z)    predicted position (x, y, z)   ball err   recorded ball (x, y, z)    predicted ball (x, y, z)");
    for row in &comparison.rows {
        if row.tick % every.max(1) != 0 {
            continue;
        }
        println!(
            "{tick:>4}  {pe:>7.1}  {ve:>7.1}  {se:>8.2}   {rp}  {pp}   {be:>8.1}   {rb}  {pb}",
            tick = row.tick,
            pe = row.position_error(),
            ve = row.velocity_error(),
            se = row.spin_error(),
            rp = fmt_vec(&row.recorded.position),
            pp = fmt_vec(&row.predicted.position),
            be = row.ball_error(),
            rb = fmt_vec(&row.recorded_ball),
            pb = fmt_vec(&row.predicted_ball),
        );
    }
    // `RB_STATES=1` lists the first car's recorded and predicted velocity and
    // spin per row, to see which component a position error starts in.
    if std::env::var_os("RB_STATES").is_some() {
        for row in &comparison.rows {
            if row.tick % every.max(1) != 0 {
                continue;
            }
            println!(
                "car 0 tick {tick:>4}: rec vel {} spin {} | pred vel {} spin {}",
                fmt_vec(&row.recorded.velocity),
                fmt_vec(&row.recorded.angular_velocity),
                fmt_vec(&row.predicted.velocity),
                fmt_vec(&row.predicted.angular_velocity),
                tick = row.tick,
            );
        }
    }
    if std::env::var_os("RB_OTHERS").is_some() {
        for (index, rows) in comparison.other_rows.iter().enumerate() {
            for (tick, recorded, predicted) in rows {
                if tick % every.max(1) != 0 {
                    continue;
                }
                println!(
                    "car {} tick {tick:>4}: rec pos {} vel {} spin {} | pred pos {} vel {} spin {}",
                    index + 1,
                    fmt_vec(&recorded.position),
                    fmt_vec(&recorded.velocity),
                    fmt_vec(&recorded.angular_velocity),
                    fmt_vec(&predicted.position),
                    fmt_vec(&predicted.velocity),
                    fmt_vec(&predicted.angular_velocity),
                );
            }
        }
    }
    let first = |threshold: f32| {
        comparison
            .first_position_error_over(threshold)
            .map_or_else(|| "never".to_string(), |tick| format!("tick {tick}"))
    };
    println!(
        "position error: mean {:.1} uu, max {:.1} uu; first over 10 uu: {}; first over 100 uu: {}",
        comparison.mean_position_error(),
        comparison.max_position_error(),
        first(10.0),
        first(100.0),
    );
    let first_ball = |threshold: f32| {
        comparison
            .first_ball_error_over(threshold)
            .map_or_else(|| "never".to_string(), |tick| format!("tick {tick}"))
    };
    println!(
        "ball error: mean {:.1} uu, max {:.1} uu; first over 10 uu: {}; first over 100 uu: {}",
        comparison.mean_ball_error(),
        comparison.max_ball_error(),
        first_ball(10.0),
        first_ball(100.0),
    );
    for index in 0..comparison.other_rows.len() {
        println!(
            "car {} error: mean {:.1} uu, max {:.1} uu",
            index + 1,
            comparison.mean_other_error(index),
            comparison.max_other_error(index),
        );
    }
    println!(
        "recorded inputs that differ from the tape: {}",
        comparison.input_mismatches
    );
    Ok(())
}

/// Cuts a scenario from a window of a capture and prints it as JSON.
fn run_scenario_from(
    capture: &str,
    from_secs: f32,
    to_secs: f32,
    name: Option<String>,
) -> Result<(), String> {
    let frames = CaptureFileSource::new(capture)
        .frames()
        .map_err(|e| format!("{capture}: {e}"))?;
    let name = name.unwrap_or_else(|| {
        let stem = Path::new(capture).file_stem().map_or_else(
            || capture.to_string(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        format!("{stem} {from_secs}-{to_secs} s")
    });
    let scenario = scenario_from_capture(&frames, from_secs, to_secs, &name)
        .map_err(|e| format!("{capture}: {e}"))?;
    let text = scenario.to_json().map_err(|e| e.to_string())?;
    println!("{text}");
    Ok(())
}

/// `--sweep-hit <k> <capture> <from> <to> [<capture> <from> <to>]...`: ranks
/// car-ball tunings over the windows (the first is the target, the rest
/// guards). Candidates only: see `sweep`'s module doc for the full gate.
fn run_sweep_hit(args: Vec<String>) -> Result<(), String> {
    let mut args = args.into_iter();
    let k = args
        .next()
        .ok_or("missing k")?
        .parse::<usize>()
        .map_err(|_| "invalid k".to_string())?;
    let rest: Vec<String> = args.collect();
    if rest.is_empty() || !rest.len().is_multiple_of(3) {
        return Err("expected <capture> <from-secs> <to-secs> triples".to_string());
    }
    let mut windows = Vec::new();
    for triple in rest.chunks(3) {
        let from = parse_secs("from-secs", Some(triple[1].clone()))?;
        let to = parse_secs("to-secs", Some(triple[2].clone()))?;
        windows.push(
            Window::from_capture(&triple[0], from, to)
                .map_err(|e| format!("{}: {e}", triple[0]))?,
        );
    }
    let grid = default_grid();
    let ranked = sweep(&windows, k, &grid);
    let describe = |c: &Candidate| {
        let per_window: Vec<String> = c
            .windows
            .iter()
            .map(|w| format!("{:.1}/{:.2}", w.car_velocity, w.ball_position))
            .collect();
        let t = &c.tuning;
        format!(
            "total {:7.2}  windows (car uu/s / ball uu) {}  rest {} fric {} scale {} z {} fwd {}",
            c.total(),
            per_window.join(" "),
            t.material.restitution,
            t.material.friction,
            t.hit_scale,
            t.hit_z_scale,
            t.hit_forward_scale
        )
    };
    if let Some(base) = ranked.iter().find(|c| c.tuning == grid[0]) {
        println!("default   {}", describe(base));
    }
    for (i, candidate) in ranked.iter().take(10).enumerate() {
        println!("rank {:2}   {}", i + 1, describe(candidate));
    }
    println!(
        "{} tunings, k = {k}. Candidates only: check any winner against the full-capture --self-kstep 30 table.",
        ranked.len()
    );
    Ok(())
}

fn parse_secs(name: &str, raw: Option<String>) -> Result<f32, String> {
    let raw = raw.ok_or_else(|| format!("missing {name}"))?;
    raw.parse::<f32>()
        .map_err(|_| format!("invalid {name}: {raw:?}"))
}

fn parse_max_timestamp_delta_secs(raw: Option<String>) -> Result<f32, String> {
    match raw {
        Some(raw) => raw
            .parse::<f32>()
            .map_err(|_| format!("invalid max-timestamp-delta-secs: {raw:?}")),
        None => Ok(DEFAULT_MAX_TIMESTAMP_DELTA_SECS),
    }
}

fn parse_window_secs(raw: Option<String>) -> Result<f32, String> {
    match raw {
        Some(raw) => raw
            .parse::<f32>()
            .map_err(|_| format!("invalid window-secs: {raw:?}")),
        None => Ok(DEFAULT_GROWTH_WINDOW_SECS),
    }
}

fn usage() -> &'static str {
    "usage:\n  rb-verify [--seed-first-frame] <mode> ...\n  rb-verify <replay-file> <capture-file> [max-timestamp-delta-secs]\n  rb-verify --self <capture-file> [max-timestamp-delta-secs]\n  rb-verify --self-growth <capture-file> [window-secs] [max-timestamp-delta-secs]\n  rb-verify --self-trace <capture-file> <from-secs> <to-secs>\n  rb-verify --self-onestep <capture-file> [count]\n  rb-verify --self-kstep <capture-file> [k] [count]\n  rb-verify --scenario <scenario.json> [every]\n  rb-verify --scenario <scenario.json> --against <capture> [every]\n  rb-verify --scenario-from <capture> <from-secs> <to-secs> [name]\n  rb-verify --sweep-hit <k> <capture> <from-secs> <to-secs> [<capture> <from-secs> <to-secs>]...\n\n  --seed-first-frame, given first, makes the --self* modes seed the simulation\n  from the capture's first frame instead of its first grounded, neutral frame\n  (for a recording whose start was set by the tape bot)."
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(mut first) = args.next() else {
        eprintln!("{}", usage());
        return ExitCode::FAILURE;
    };
    let mut seed_frame = SeedFrame::default();
    if first == "--seed-first-frame" {
        seed_frame = SeedFrame::First;
        let Some(mode) = args.next() else {
            eprintln!("{}", usage());
            return ExitCode::FAILURE;
        };
        first = mode;
    }

    if first == "--sweep-hit" {
        return match run_sweep_hit(args.collect()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}\n{}", usage());
                ExitCode::FAILURE
            }
        };
    }

    if first == "--scenario-from" {
        let Some(capture) = args.next() else {
            eprintln!("{}", usage());
            return ExitCode::FAILURE;
        };
        let window = parse_secs("from-secs", args.next())
            .and_then(|from| parse_secs("to-secs", args.next()).map(|to| (from, to)));
        let (from_secs, to_secs) = match window {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}\n{}", usage());
                return ExitCode::FAILURE;
            }
        };
        return match run_scenario_from(&capture, from_secs, to_secs, args.next()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }

    if first == "--scenario" {
        let Some(path) = args.next() else {
            eprintln!("{}", usage());
            return ExitCode::FAILURE;
        };
        let mut capture = None;
        let mut recorded_inputs = false;
        let mut next = args.next();
        if next.as_deref() == Some("--against") {
            let Some(capture_path) = args.next() else {
                eprintln!("{}", usage());
                return ExitCode::FAILURE;
            };
            capture = Some(capture_path);
            next = args.next();
            if next.as_deref() == Some("--recorded-inputs") {
                recorded_inputs = true;
                next = args.next();
            }
        }
        let every = match next.map(|raw| raw.parse::<usize>()) {
            None => 12,
            Some(Ok(every)) => every,
            Some(Err(_)) => {
                eprintln!("invalid every\n{}", usage());
                return ExitCode::FAILURE;
            }
        };
        let result = match capture {
            None => run_scenario(&path, every),
            Some(capture) => run_scenario_against(&path, &capture, every, recorded_inputs),
        };
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }

    if first == "--self-trace" {
        let Some(capture_path) = args.next() else {
            eprintln!("{}", usage());
            return ExitCode::FAILURE;
        };
        let window = parse_secs("from-secs", args.next())
            .and_then(|from| parse_secs("to-secs", args.next()).map(|to| (from, to)));
        let (from_secs, to_secs) = match window {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}\n{}", usage());
                return ExitCode::FAILURE;
            }
        };
        return match trace_capture(capture_path, seed_frame, from_secs, to_secs) {
            Err(e) => {
                eprintln!("ingestion failed: {e}");
                ExitCode::FAILURE
            }
            Ok(rows) => {
                print_trace(&rows);
                ExitCode::SUCCESS
            }
        };
    }

    if first == "--self-kstep" {
        let Some(capture_path) = args.next() else {
            eprintln!("{}", usage());
            return ExitCode::FAILURE;
        };
        let k = match args.next().map(|raw| raw.parse::<usize>()) {
            None => DEFAULT_K_STEP,
            Some(Ok(k)) if k > 0 => k,
            Some(_) => {
                eprintln!("invalid k (a positive tick count)\n{}", usage());
                return ExitCode::FAILURE;
            }
        };
        let count = match args.next().map(|raw| raw.parse::<usize>()) {
            None => 0,
            Some(Ok(count)) => count,
            Some(Err(_)) => {
                eprintln!("invalid count\n{}", usage());
                return ExitCode::FAILURE;
            }
        };
        let scored = k_step_score(
            &capture_path,
            seed_frame,
            k,
            DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
        )
        .and_then(|score| Ok((score, k_step_capture(&capture_path, seed_frame, k)?)));
        return match scored {
            Err(e) => {
                eprintln!("ingestion failed: {e}");
                ExitCode::FAILURE
            }
            Ok((score, rows)) => {
                print_score(&score);
                print_worst_steps(&rows, count);
                ExitCode::SUCCESS
            }
        };
    }

    if first == "--self-onestep" {
        let Some(capture_path) = args.next() else {
            eprintln!("{}", usage());
            return ExitCode::FAILURE;
        };
        let count = match args.next().map(|raw| raw.parse::<usize>()) {
            None => 20,
            Some(Ok(count)) => count,
            Some(Err(_)) => {
                eprintln!("invalid count\n{}", usage());
                return ExitCode::FAILURE;
            }
        };
        return match one_step_capture(capture_path, seed_frame) {
            Err(e) => {
                eprintln!("ingestion failed: {e}");
                ExitCode::FAILURE
            }
            Ok(rows) => {
                print_worst_steps(&rows, count);
                ExitCode::SUCCESS
            }
        };
    }

    if first == "--self-growth" {
        let Some(capture_path) = args.next() else {
            eprintln!("{}", usage());
            return ExitCode::FAILURE;
        };
        let window_secs = match parse_window_secs(args.next()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let max_timestamp_delta_secs = match parse_max_timestamp_delta_secs(args.next()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        return match score_capture_growth(
            capture_path,
            seed_frame,
            max_timestamp_delta_secs,
            window_secs,
        ) {
            Err(e) => {
                eprintln!("ingestion failed: {e}");
                ExitCode::FAILURE
            }
            Ok(windows) => {
                print_growth(&windows);
                ExitCode::SUCCESS
            }
        };
    }

    let result = if first == "--self" {
        let Some(capture_path) = args.next() else {
            eprintln!("{}", usage());
            return ExitCode::FAILURE;
        };
        let max_timestamp_delta_secs = match parse_max_timestamp_delta_secs(args.next()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        score_capture_against_candidate(capture_path, seed_frame, max_timestamp_delta_secs)
    } else {
        let Some(capture_path) = args.next() else {
            eprintln!("{}", usage());
            return ExitCode::FAILURE;
        };
        let max_timestamp_delta_secs = match parse_max_timestamp_delta_secs(args.next()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        score_replay_against_capture(first, capture_path, max_timestamp_delta_secs)
    };

    match result {
        Err(e) => {
            eprintln!("ingestion failed: {e}");
            ExitCode::FAILURE
        }
        Ok(score) => {
            print_score(&score);
            ExitCode::SUCCESS
        }
    }
}

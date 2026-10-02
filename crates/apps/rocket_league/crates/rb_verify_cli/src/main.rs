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
//! - `rb-verify --self-kstep <capture-file> [k] [count]`: every frame
//!   predicted `k` ticks ahead (default 30) from the recorded frame `k`
//!   before it, scored like `--self` (`k_step_score`,
//!   `RB-VERIFY-003-FR-008`), then the `count` (default 0) frames with the
//!   largest car velocity error (`k_step_capture`).

use rb_domain::divergence::DivergenceScore;
use rb_domain::{ControllerInput, Vec3};
use rb_verify_cli::{
    car_frame_spin, k_step_capture, k_step_score, one_step_capture, rotation_rate,
    score_capture_against_candidate, score_capture_growth, score_replay_against_capture,
    trace_capture, TraceRow, DEFAULT_GROWTH_WINDOW_SECS, DEFAULT_K_STEP,
    DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
};
use std::env;
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
    "usage:\n  rb-verify <replay-file> <capture-file> [max-timestamp-delta-secs]\n  rb-verify --self <capture-file> [max-timestamp-delta-secs]\n  rb-verify --self-growth <capture-file> [window-secs] [max-timestamp-delta-secs]\n  rb-verify --self-trace <capture-file> <from-secs> <to-secs>\n  rb-verify --self-onestep <capture-file> [count]\n  rb-verify --self-kstep <capture-file> [k] [count]"
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(first) = args.next() else {
        eprintln!("{}", usage());
        return ExitCode::FAILURE;
    };

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
        return match trace_capture(capture_path, from_secs, to_secs) {
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
        let scored = k_step_score(&capture_path, k, DEFAULT_MAX_TIMESTAMP_DELTA_SECS)
            .and_then(|score| Ok((score, k_step_capture(&capture_path, k)?)));
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
        return match one_step_capture(capture_path) {
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
        return match score_capture_growth(capture_path, max_timestamp_delta_secs, window_secs) {
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
        score_capture_against_candidate(capture_path, max_timestamp_delta_secs)
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

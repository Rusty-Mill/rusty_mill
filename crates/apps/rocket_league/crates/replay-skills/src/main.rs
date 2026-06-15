//! CLI: detect mechanical skills in a `.replay` and verify whether skills were
//! performed.
//!
//! - default: print a per-player skill table (optionally one `--player`).
//! - `--verify <skill>`: answer "was this skill performed?" — exit code 0 if so,
//!   2 if not — so it is usable as a scripting gate. Scope to a `--player` and/or
//!   a `--window <start> <end>` to verify within a clip.
//! - `--list`: print the skill catalog.
//! - `--json <out>`: write the full [`SkillReport`] as JSON.

use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_skills::{detect_all, Skill, SkillConfig, SkillReport};

const USAGE: &str = "usage: replay-skills <file.replay> [--player <name>] \
[--verify <skill>] [--window <start_s> <end_s>] [--config <cfg.json>] \
[--json <out.json>] [--list]";

/// Exit code when `--verify` finds the skill was *not* performed (for scripting).
const NOT_PERFORMED: u8 = 2;

struct Args {
    replay: Option<String>,
    player: Option<String>,
    verify: Option<String>,
    window: Option<(f32, f32)>,
    config: Option<String>,
    json: Option<String>,
    list: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        replay: None,
        player: None,
        verify: None,
        window: None,
        config: None,
        json: None,
        list: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--player" => a.player = Some(it.next().ok_or("--player needs a name")?),
            "--verify" => a.verify = Some(it.next().ok_or("--verify needs a skill key")?),
            "--window" => {
                let s = it.next().ok_or("--window needs <start> <end>")?;
                let e = it.next().ok_or("--window needs <start> <end>")?;
                let s: f32 = s.parse().map_err(|_| "window start must be a number")?;
                let e: f32 = e.parse().map_err(|_| "window end must be a number")?;
                a.window = Some((s, e));
            }
            "--config" => a.config = Some(it.next().ok_or("--config needs a path")?),
            "--json" => a.json = Some(it.next().ok_or("--json needs a path")?),
            "--list" => a.list = true,
            "-h" | "--help" => return Err(USAGE.to_string()),
            other if other.starts_with('-') => {
                return Err(format!("unknown flag {other}\n{USAGE}"))
            }
            other if a.replay.is_none() => a.replay = Some(other.to_string()),
            other => return Err(format!("unexpected arg {other}\n{USAGE}")),
        }
    }
    Ok(a)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, Box<dyn Error>> {
    let args = parse_args().map_err(|e| -> Box<dyn Error> { e.into() })?;

    if args.list {
        print_catalog();
        // `--list` is informational; still proceed if a replay was also given.
        if args.replay.is_none() {
            return Ok(ExitCode::SUCCESS);
        }
    }

    let replay = args.replay.clone().ok_or(USAGE)?;
    let data = std::fs::read(&replay)?;
    let decoded = BoxcarsParser::new().parse(&data)?;
    let replay_id = Path::new(&replay)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("replay")
        .to_string();
    let canonical = build_canonical(&decoded, replay_id);

    let cfg: SkillConfig = match &args.config {
        Some(p) => serde_json::from_slice(&std::fs::read(p)?)?,
        None => SkillConfig::default(),
    };
    let report = detect_all(&canonical, &cfg);

    let code = match &args.verify {
        Some(key) => verify(&report, key, &args)?,
        None => {
            print_summary(&report, args.player.as_deref());
            ExitCode::SUCCESS
        }
    };

    if let Some(path) = &args.json {
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
        eprintln!("\nwrote skill report -> {path}");
    }
    Ok(code)
}

/// Answer a `--verify <skill>` query and map it to an exit code.
fn verify(report: &SkillReport, key: &str, args: &Args) -> Result<ExitCode, Box<dyn Error>> {
    let skill = Skill::from_key(key).ok_or_else(|| {
        let keys: Vec<&str> = Skill::ALL.iter().map(|s| s.key()).collect();
        format!("unknown skill '{key}'. valid: {}", keys.join(", "))
    })?;

    // Resolve an optional player filter to a PRI.
    let pri = match &args.player {
        Some(name) => Some(
            report
                .player_by_name(name)
                .ok_or_else(|| format!("player '{name}' not found in replay"))?
                .pri,
        ),
        None => None,
    };

    let performed = match (pri, args.window) {
        (Some(pri), Some((s, e))) => report.performed_by_in_window(skill, pri, s, e),
        (Some(pri), None) => report.performed_by(skill, pri),
        (None, Some((s, e))) => report.performed_in_window(skill, s, e),
        (None, None) => report.performed(skill),
    };

    let who = args.player.as_deref().unwrap_or("anyone");
    let when = match args.window {
        Some((s, e)) => format!(" in [{s:.1}s, {e:.1}s]"),
        None => String::new(),
    };
    // Count within the same scope as the YES/no answer (player and/or window).
    let count = report
        .instances
        .iter()
        .filter(|i| {
            i.skill == skill
                && pri.is_none_or(|p| i.pri == p)
                && args.window.is_none_or(|(s, e)| i.t >= s && i.t <= e)
        })
        .count();
    println!(
        "{}: {} performed {}{} ({} occurrence{})",
        skill.display_name(),
        who,
        if performed { "YES" } else { "no" },
        when,
        count,
        if count == 1 { "" } else { "s" },
    );

    Ok(if performed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(NOT_PERFORMED)
    })
}

fn print_catalog() {
    println!("Skill catalog ({} skills):", Skill::ALL.len());
    for s in Skill::ALL {
        println!(
            "  {:<20} [{:?}/{:?}]  {}",
            s.key(),
            s.category(),
            s.detection(),
            s.description(),
        );
    }
}

fn print_summary(report: &SkillReport, player_filter: Option<&str>) {
    eprintln!("== SKILL REPORT ==");
    eprintln!("replay_id : {}", report.replay_id);
    eprintln!("config    : {}", report.config_version);
    eprintln!("parser    : {}", report.parser_version);
    eprintln!("instances : {}", report.instances.len());

    for p in &report.players {
        if let Some(name) = player_filter {
            if p.player != name {
                continue;
            }
        }
        eprintln!("\n-- {} (pri {}, team {:?}) --", p.player, p.pri, p.team);
        if p.counts.is_empty() {
            eprintln!("  (no catalogued skills detected)");
            continue;
        }
        for (skill, n) in &p.counts {
            eprintln!("  {:<20} x{}", skill.display_name(), n);
        }
    }
}

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
use replay_analyzer::model::CanonicalMatch;
use replay_skills::{detect_all, skill_values, Skill, SkillConfig, SkillReport, TouchDv};
use replay_value::{build_dataset, per_touch_delta_v, ValueConfig, ValueModel, ValuePredictor};

const USAGE: &str = "usage: replay-skills <file.replay> [--player <name>] \
[--verify <skill>] [--window <start_s> <end_s>] [--profile] [--outcomes] \
[--value] [--value-model <model.json>] [--config <cfg.json>] [--json <out.json>] \
[--list]";

/// Exit code when `--verify` finds the skill was *not* performed (for scripting).
const NOT_PERFORMED: u8 = 2;

/// Window (s) a skill may precede a same-team goal to count toward its buildup.
const OUTCOME_WINDOW_S: f32 = 6.0;

/// Default pretrained (corpus-fit) value-model artifact for `--value`.
const DEFAULT_VALUE_MODEL: &str = "assets/corpus/value_model.json";

/// Max time gap (s) to link a skill instance to a touch's ΔV — touch-based
/// skills are emitted exactly at the touch time, so this only needs slack for
/// float jitter, not to bridge to unrelated touches.
const VALUE_LINK_TOL_S: f32 = 0.1;

struct Args {
    replay: Option<String>,
    player: Option<String>,
    verify: Option<String>,
    window: Option<(f32, f32)>,
    config: Option<String>,
    json: Option<String>,
    list: bool,
    profile: bool,
    outcomes: bool,
    value: bool,
    value_model: Option<String>,
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
        profile: false,
        outcomes: false,
        value: false,
        value_model: None,
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
            "--profile" => a.profile = true,
            "--outcomes" => a.outcomes = true,
            "--value" => a.value = true,
            "--value-model" => a.value_model = Some(it.next().ok_or("--value-model needs a path")?),
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
            if args.profile {
                print_profiles(&report, canonical.duration_s, args.player.as_deref());
            } else if args.outcomes {
                print_outcomes(&report, &canonical.events, args.player.as_deref());
            } else if args.value {
                print_value(
                    &report,
                    &canonical,
                    args.value_model.as_deref(),
                    args.player.as_deref(),
                );
            } else {
                print_summary(&report, args.player.as_deref());
            }
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

/// Per-player proficiency: counts, rate per minute, and a quality proxy.
fn print_profiles(report: &SkillReport, duration_s: f32, player_filter: Option<&str>) {
    eprintln!(
        "== SKILL PROFILE ==  (~{:.0}s · q = mean confidence · last column = mean evidence magnitude)",
        duration_s
    );
    for p in replay_skills::profiles(report, duration_s) {
        if let Some(name) = player_filter {
            if p.player != name {
                continue;
            }
        }
        eprintln!(
            "\n-- {} (pri {}, team {:?}) --  {:.2} skills/min",
            p.player, p.pri, p.team, p.total_per_min
        );
        if p.skills.is_empty() {
            eprintln!("  (no catalogued skills detected)");
            continue;
        }
        for (skill, st) in &p.skills {
            eprintln!(
                "  {:<20} x{:<3} {:>5.2}/min  q{:.2}  {}",
                skill.display_name(),
                st.count,
                st.per_min,
                st.mean_quality,
                fmt_metric(*skill, st.mean_metric),
            );
        }
    }
}

/// Render a skill's mean structured metric with its unit (seconds get decimals,
/// the rest round to whole units). Empty for skills with no labelled magnitude.
fn fmt_metric(skill: Skill, value: f32) -> String {
    let label = skill.metric_label();
    if label.is_empty() {
        return String::new();
    }
    let unit = skill.metric_unit();
    if unit == "s" {
        format!("{label} {value:.2}{unit}")
    } else {
        format!("{label} {value:.0}{unit}")
    }
}

/// Per-player skill→goal involvement: reps within the window before a same-team goal.
fn print_outcomes(
    report: &SkillReport,
    events: &[replay_analyzer::model::Event],
    filter: Option<&str>,
) {
    eprintln!(
        "== SKILL OUTCOMES ==  (reps within {:.0}s before a same-team goal)",
        OUTCOME_WINDOW_S
    );
    for o in replay_skills::outcomes(report, events, OUTCOME_WINDOW_S) {
        if let Some(name) = filter {
            if o.player != name {
                continue;
            }
        }
        eprintln!(
            "\n-- {} (pri {}, team {:?}) --  {}/{} skills in goal buildups",
            o.player, o.pri, o.team, o.total_before_goal, o.total
        );
        for (skill, (total, conv)) in &o.by_skill {
            if *conv > 0 {
                eprintln!(
                    "  {:<20} {}/{} before a goal",
                    skill.display_name(),
                    conv,
                    total
                );
            }
        }
    }
}

/// Per-player skill→value (ΔV) view: how much each ball-contact skill moved the
/// team's scoring probability, via the value model's per-touch swing.
fn print_value(
    report: &SkillReport,
    canonical: &CanonicalMatch,
    model_path: Option<&str>,
    filter: Option<&str>,
) {
    let vcfg = ValueConfig::default();
    let model = load_value_model(model_path.unwrap_or(DEFAULT_VALUE_MODEL), canonical, &vcfg);
    let touch_dv: Vec<TouchDv> = per_touch_delta_v(canonical, &model, &vcfg)
        .into_iter()
        .map(|tv| TouchDv {
            pri: tv.pri,
            t: tv.t,
            dv: tv.dv,
        })
        .collect();

    eprintln!("== SKILL VALUE ==  (mean ΔV = scoring-prob swing per rep; + helps, - hurts)");
    for v in skill_values(report, &touch_dv, VALUE_LINK_TOL_S) {
        if let Some(name) = filter {
            if v.player != name {
                continue;
            }
        }
        if v.linked == 0 {
            continue;
        }
        eprintln!(
            "\n-- {} (pri {}, team {:?}) --  ΔV {:+.3} over {} linked rep{}",
            v.player,
            v.pri,
            v.team,
            v.sum_dv,
            v.linked,
            if v.linked == 1 { "" } else { "s" },
        );
        for (skill, (n, sum)) in &v.by_skill {
            eprintln!(
                "  {:<20} x{:<3} ΔV {:+.3}/rep",
                skill.display_name(),
                n,
                sum / *n as f32,
            );
        }
    }
}

/// Load the pretrained (corpus-fit) value model from `path`; if it can't be read,
/// fall back to a per-match model trained on this single replay (weak, but keeps
/// `--value` working) and note it on stderr.
fn load_value_model(path: &str, canonical: &CanonicalMatch, vcfg: &ValueConfig) -> ValuePredictor {
    match std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<ValuePredictor>(&b).ok())
    {
        Some(m) => {
            eprintln!("(value model: {path}, trained on {} states)", m.n_train());
            m
        }
        None => {
            let ds = build_dataset(canonical, vcfg);
            let m = ValueModel::train(&ds, &vcfg.train);
            eprintln!(
                "(no value model at {path}; trained per-match on {} states — single-replay, weak)",
                m.n_train
            );
            ValuePredictor::Logistic(m)
        }
    }
}

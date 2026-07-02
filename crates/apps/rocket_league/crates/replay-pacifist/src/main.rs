//! CLI: parse a `.replay` via the analyzer, bridge to a [`Timeline`], and print
//! each player's Pacifist Score with its per-dimension breakdown.

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_pacifist::bridge::{possession_spans, timeline_from_canonical};
use replay_pacifist::context::MatchContext;
use replay_pacifist::scoring::Analyzer;
use replay_pacifist::severity::Severity;
use replay_pacifist::{roster, team_sizes, Team, PACIFIST_CONFIG_VERSION};
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: replay-pacifist <file.replay>";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let path = std::env::args().nth(1).ok_or(USAGE)?;
    let data = std::fs::read(&path)?;
    let decoded = BoxcarsParser::new().parse(&data)?;
    let replay_id = Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("replay")
        .to_string();
    let canonical = build_canonical(&decoded, replay_id);

    let timeline = timeline_from_canonical(&canonical);
    let sizes = team_sizes(&timeline);
    if !sizes.is_two_v_two() {
        eprintln!(
            "warning: {}v{} — the Pacifist dimensions are tuned for 2v2; scores are indicative only",
            sizes.blue, sizes.orange
        );
    }

    let analyzer = Analyzer::default();
    let spans = possession_spans(&canonical);
    let ctx = MatchContext::derive_with_possession(&timeline, analyzer.context_config(), &spans);

    eprintln!(
        "config={PACIFIST_CONFIG_VERSION}  possession=touch-decoded ({} runs)",
        spans.len()
    );
    let counts = ctx.possession_counts();
    let total = (counts.blue + counts.orange + counts.contested).max(1) as f32;
    eprintln!(
        "possession: blue {:.0}%  orange {:.0}%  contested {:.0}%",
        counts.blue as f32 / total * 100.0,
        counts.orange as f32 / total * 100.0,
        counts.contested as f32 / total * 100.0,
    );

    for entry in roster(&timeline) {
        let score = analyzer.score_player_in(&ctx, entry.player);
        let side = match entry.team {
            Team::Blue => "blue",
            Team::Orange => "orange",
        };
        let name = entry.name.as_deref().unwrap_or("<unknown>");
        match score.value {
            Some(v) => eprintln!(
                "\n== {name} ({side}) ==  pacifist score {:.1}  (confidence {:.2})  {} ({} minor, {} major)",
                v.get(),
                score.confidence.get(),
                score.faults.verdict.label(),
                score.faults.minors,
                score.faults.majors,
            ),
            None => eprintln!("\n== {name} ({side}) ==  no scoreable evidence"),
        }
        for f in score
            .faults
            .faults
            .iter()
            .filter(|f| f.severity == Severity::Major)
        {
            eprintln!("  MAJOR @{:>6.1}s  [{}] {}", f.t, f.criterion, f.detail);
        }
        for d in &score.breakdown {
            eprintln!(
                "  {:<22} value={:>5.1}  conf={:.2}  influence={:>4.1}%  evidence={}",
                d.dimension.label(),
                d.value.get(),
                d.confidence.get(),
                d.influence * 100.0,
                d.evidence.len()
            );
        }
    }
    Ok(())
}

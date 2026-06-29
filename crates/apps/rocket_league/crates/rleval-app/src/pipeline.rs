//! The unified analysis pipeline.
//!
//! This is the single place the formerly-disparate executables are folded into
//! one in-process flow: decode → canonical model → score → skills → value →
//! 3D scene. It runs the identical pure cores the individual CLIs do (no
//! subprocess shelling), and returns one [`Analysis`] bundling every view —
//! the structured results plus the two self-contained HTML documents (the 3D
//! viewer and the scoring report) the web UI embeds side by side.

use std::error::Error;

use replay_analyzer::analyze::bcstats::{ballchasing_stats, BcPlayerStats};
use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_analyzer::field::is_standard_geometry;

use replay_scoring::heatmap::{occupancy, render_svg, touch_points};
use replay_scoring::lobby::assemble;
use replay_scoring::render::html as scoring_html;
use replay_scoring::{score_all, Report, ScoreConfig};

use replay_skills::profile::{profiles, PlayerSkillProfile};
use replay_skills::{detect_all, SkillConfig, SkillReport};

use replay_value::{evaluate, PlayerValue, ValueConfig};

use replay_viewer::{
    attach_impact, attach_player_stats, attach_roles, attach_winprob, build_scene, html_offline,
};

use serde::Serialize;

/// Everything the unified UI needs for one replay, computed in a single pass.
#[derive(Serialize)]
pub struct Analysis {
    // ---- match summary ----
    pub replay_id: String,
    pub map: Option<String>,
    pub team_size: Option<i32>,
    pub duration_s: f32,
    pub team_scores: Vec<(i32, i32)>,
    /// `false` for Hoops/Dropshot/etc. — the UI surfaces a low-confidence banner.
    pub standard_map: bool,

    // ---- structured per-engine results ----
    /// Decision-discipline scoring, one report per player (`replay-scoring`).
    pub scores: Vec<Report>,
    /// Mechanical skill proficiency per player (`replay-skills`).
    pub skill_profiles: Vec<PlayerSkillProfile>,
    /// Per-player value impact (ΔV) — the independent validator (`replay-value`).
    pub impact: ImpactSummary,
    /// Ballchasing-parity aggregates per player — boost / movement / positioning /
    /// demo (`analyze::bcstats`), the full stat surface for the Stats tab.
    pub bc_stats: Vec<BcPlayerStats>,

    // ---- self-contained embeddable views ----
    /// The full 3D replay viewer as a self-contained (offline) HTML document.
    pub viewer_html: String,
    /// The lobby scoring report as a self-contained HTML document.
    pub scoring_html: String,
}

/// Per-player value impact plus the model fit context.
#[derive(Serialize)]
pub struct ImpactSummary {
    pub base_rate: f32,
    pub log_loss: f32,
    pub players: Vec<PlayerValue>,
}

/// Run the whole pipeline on raw `.replay` bytes, returning the unified bundle.
///
/// `replay_id` labels the match (normally the upload's file stem). Every engine
/// runs at its default config — the same defaults the standalone CLIs use.
pub fn analyze(bytes: &[u8], replay_id: &str) -> Result<Analysis, Box<dyn Error>> {
    // 1. Decode + reconstruct the neutral canonical model — the shared contract.
    let decoded = BoxcarsParser::new().parse(bytes)?;
    let canonical = build_canonical(&decoded, replay_id.to_string());
    let standard_map = is_standard_geometry(canonical.map.as_deref());

    // 2. Decision-discipline scoring (per player) + the lobby report HTML.
    let score_cfg = ScoreConfig::default();
    let scores = score_all(&canonical, &score_cfg);
    let lobby = assemble(&canonical, &score_cfg);
    let heatmaps: Vec<(i32, String)> = lobby
        .players
        .iter()
        .map(|p| {
            let occ = occupancy(&canonical, p.target_pri, 12, 15);
            let touches = touch_points(&canonical, p.target_pri);
            (p.target_pri, render_svg(&occ, &touches))
        })
        .collect();
    let scoring_html = scoring_html(&lobby, &heatmaps);

    // 3. Mechanical skills: detect, then distil per-player proficiency profiles.
    let skill_report: SkillReport = detect_all(&canonical, &SkillConfig::default());
    let skill_profiles = profiles(&skill_report, canonical.duration_s);

    // 4. Value model (ΔV) — trained in-process on this match, no model file.
    let value_cfg = ValueConfig::default();
    let evaluation = evaluate(&canonical, &value_cfg);
    let impact = ImpactSummary {
        base_rate: evaluation.base_rate,
        log_loss: evaluation.log_loss,
        players: evaluation.players,
    };

    // 5. Ballchasing-parity aggregate block (boost / movement / positioning / demo).
    let bc_stats = ballchasing_stats(&canonical);

    // 6. 3D scene with every overlay (roles / win-prob / impact / player stats),
    //    serialized into a self-contained offline viewer document.
    let mut scene = build_scene(&canonical, &skill_report.instances);
    attach_roles(&mut scene, &canonical, &score_cfg);
    attach_winprob(&mut scene, &canonical, &value_cfg);
    attach_impact(&mut scene, &canonical, &value_cfg);
    attach_player_stats(&mut scene, &canonical);
    let viewer_html = html_offline(&scene);

    Ok(Analysis {
        replay_id: canonical.replay_id.clone(),
        map: canonical.map.clone(),
        team_size: canonical.team_size,
        duration_s: canonical.duration_s,
        team_scores: canonical
            .team_scores
            .iter()
            .map(|(&k, &v)| (k, v))
            .collect(),
        standard_map,
        scores,
        skill_profiles,
        impact,
        bc_stats,
        viewer_html,
        scoring_html,
    })
}

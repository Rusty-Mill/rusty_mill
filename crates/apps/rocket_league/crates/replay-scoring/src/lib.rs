//! Decision-discipline scoring over the canonical match model.
//!
//! A **pure** consumer of [`replay_analyzer`]'s [`CanonicalMatch`]: no parsing,
//! no I/O, no clock. The whole engine is `score(match, target, config) → Report`,
//! so it is deterministic, golden-testable, and re-runnable at a new config
//! version without re-parsing. Scoring logic lives *only* here — never in the
//! analyzer (the ports-and-adapters seam).
//!
//! Scores are **heuristic** decision-discipline ratings inferred from kinematics,
//! and the default [`ScoreConfig`] bands are pre-calibration guesses; trustworthy
//! numbers require a labeled corpus (see the design spec §0/§13).

pub mod calibrate;
pub mod chains;
pub mod config;
pub mod contract;
pub mod coverage;
pub mod engine;
pub mod episodes;
pub mod features;
pub mod heatmap;
pub mod lobby;
pub mod metrics;
pub mod reconcile;
pub mod relative;
pub mod render;
pub mod report;
pub mod roles;
pub mod xg;

use std::collections::BTreeSet;

use replay_analyzer::model::CanonicalMatch;

pub use chains::{chains, ChainEnd};
pub use config::{ScoreConfig, SCORE_CONFIG_VERSION};
pub use contract::{cross_check, BallchasingReplay, CrossCheckReport};
pub use episodes::{extract, extract_with, Episode};
pub use relative::{attach_relative, BucketNorm, RankNorms, RelativeReport};
pub use report::{Confidence, MetricBreakdown, Report};
pub use xg::{level_of, XgModel};

/// The teams present, sorted.
pub(crate) fn teams(m: &CanonicalMatch) -> Vec<i32> {
    let set: BTreeSet<i32> = m.tracks.iter().filter_map(|t| t.team).collect();
    set.into_iter().collect()
}

/// Score one target player (by stable PRI) against `cfg`.
pub fn score(m: &CanonicalMatch, target_pri: i32, cfg: &ScoreConfig) -> Report {
    let target = m.tracks.iter().find(|t| t.pri == target_pri);
    let target_player = target
        .map(|t| t.player.clone())
        .unwrap_or_else(|| "<unknown>".into());
    let target_team = target.and_then(|t| t.team);
    let team = target_team.unwrap_or(0);

    let frames = features::build_frames(m, cfg);
    let roles = roles::assign(&frames, &teams(m), cfg);
    let raws = metrics::compute(&frames, &roles, &m.events, target_pri, team, cfg);

    let valid_frames = frames
        .iter()
        .filter(|f| f.car(target_pri).map(|c| c.valid_pos).unwrap_or(false))
        .count();

    let bds = engine::breakdowns(cfg, &raws);
    let subs = engine::sub_scores(cfg, &bds);
    let composite = engine::composite(cfg, subs);
    let licence = engine::band(cfg, composite);
    let player_type = engine::classify(cfg, &raws);
    let (main_leak, focus_chapter) = engine::pick_leak(cfg, &bds);

    // Low confidence if too few analyzable frames, any sub-score had no data
    // (e.g. a missing/AFK teammate makes support metrics meaningless, §11), the
    // map is non-standard geometry (positional metrics assume standard Soccar),
    // or a lobby-mate (either team) left/went AFK for a real stretch — that
    // doesn't just wreck their own report, it turns part of the match into an
    // unrepresentative man-advantage for everyone else in it too.
    let standard_map = replay_analyzer::field::is_standard_geometry(m.map.as_deref());
    let fully_present =
        coverage::lobby_fully_present(&m.tracks, &m.events, m.duration_s, &cfg.coverage);
    let confidence = if !standard_map
        || valid_frames < cfg.min_sample_frames
        || subs.iter().any(Option::is_none)
        || !fully_present
    {
        Confidence::LowConfidence
    } else {
        Confidence::Ok
    };

    Report {
        replay_id: m.replay_id.clone(),
        score_config_version: cfg.version.clone(),
        parser_version: m.parser_version.clone(),
        analyzer_version: m.analyzer_version.clone(),
        target_player,
        target_pri,
        target_team,
        composite,
        first_man: subs[0].unwrap_or(0.0),
        second_man: subs[1].unwrap_or(0.0),
        general: subs[2].unwrap_or(0.0),
        licence,
        player_type,
        main_leak,
        focus_chapter,
        confidence,
        metrics: bds,
        // Absolute by default; the rank-relative layer is attached separately
        // (it needs the whole lobby + a corpus norms artifact).
        relative: None,
    }
}

/// Score by player name (first matching track), if present.
pub fn score_by_name(m: &CanonicalMatch, name: &str, cfg: &ScoreConfig) -> Option<Report> {
    let pri = m.tracks.iter().find(|t| t.player == name)?.pri;
    Some(score(m, pri, cfg))
}

/// Score every player that has a coalesced track.
pub fn score_all(m: &CanonicalMatch, cfg: &ScoreConfig) -> Vec<Report> {
    m.tracks.iter().map(|t| score(m, t.pri, cfg)).collect()
}

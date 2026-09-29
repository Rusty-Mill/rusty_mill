//! The unified analysis pipeline.
//!
//! This is the single place the formerly-disparate executables are folded into
//! one in-process flow: decode → canonical model → score → skills → value →
//! 3D scene. It runs the identical pure cores the individual CLIs do (no
//! subprocess shelling), and returns one [`Analysis`] bundling every view —
//! the structured results plus the two self-contained HTML documents (the 3D
//! viewer and the scoring report) the web UI embeds side by side.

use std::error::Error;
use std::path::Path;

use replay_analyzer::analyze::bcstats::{ballchasing_stats, BcPlayerStats};
use replay_analyzer::analyze::build_canonical;
use replay_analyzer::analyze::coords::{coordinate_report, CoordinateIssue};
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_analyzer::field::is_standard_geometry;

use replay_pacifist::bridge as pacifist_bridge;
use replay_pacifist::context::MatchContext as PacifistContext;
use replay_pacifist::scoring::Analyzer as PacifistAnalyzer;
use replay_pacifist::severity::{Severity, Verdict};

use replay_scoring::heatmap::{occupancy, render_svg, touch_points};
use replay_scoring::lobby::assemble;
use replay_scoring::render::html as scoring_html;
use replay_scoring::{attach_relative, score_all, RankNorms, Report, ScoreConfig};

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
    /// Human-readable coordinate-sanity findings (blue spawning on the wrong
    /// side, positions outside the arena). Empty for a healthy replay; the UI
    /// shows them as a banner because they would otherwise corrupt position
    /// scores silently.
    pub coordinate_warnings: Vec<String>,

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
    /// Core scoreboard stats per player (header truth + recomputed shooting %),
    /// for the Stats tab's Core section.
    pub core: Vec<CorePlayerStats>,
    /// Pacifist system-adherence scores per player (`replay-pacifist`) — the
    /// eight-dimension rubric plus the FM-1 fault ledger and verdict.
    pub pacifist: PacifistSummary,

    // ---- self-contained embeddable views ----
    /// The full 3D replay viewer as a self-contained (offline) HTML document.
    pub viewer_html: String,
    /// The lobby scoring report as a self-contained HTML document.
    pub scoring_html: String,
    /// Self-contained ballchasing-style stats dashboard (bc-clone).
    pub ballchasing_html: String,
}

/// Core scoreboard stats for one player — header truth (goals/assists/saves/
/// shots/score) plus a recomputed shooting % (goals/shots). Keyed by `pri`.
#[derive(Serialize)]
pub struct CorePlayerStats {
    pub pri: i32,
    pub player: String,
    pub team: Option<i32>,
    pub goals: i32,
    pub assists: i32,
    pub saves: i32,
    pub shots: i32,
    pub score: i32,
    /// goals / shots × 100; 0 when the player took no shots.
    pub shooting_pct: f32,
}

/// Per-player value impact plus the model fit context.
#[derive(Serialize)]
pub struct ImpactSummary {
    pub base_rate: f32,
    pub log_loss: f32,
    pub players: Vec<PlayerValue>,
}

/// The Pacifist tab's data: config version + one row per player.
#[derive(Serialize)]
pub struct PacifistSummary {
    /// `PACIFIST_CONFIG_VERSION` — surfaced so the UI can label the rubric.
    pub config_version: String,
    pub players: Vec<PacifistPlayer>,
}

/// One player's Pacifist score, verdict, and breakdown, flattened for the UI.
#[derive(Serialize)]
pub struct PacifistPlayer {
    pub player: String,
    /// Stable platform identity (`steam:…`) when the replay carries one and the
    /// display name is unambiguous within the match.
    pub platform_id: Option<String>,
    pub team: i32,
    /// Headline 0–100 (Major-capped); `None` when nothing applied.
    pub value: Option<f32>,
    pub confidence: f32,
    /// FM-1 driving-test verdict: `"PASS"` / `"FAIL"`.
    pub verdict: String,
    pub minors: u32,
    pub majors: u32,
    /// The Major faults only — each one is an instant failure, so each is
    /// worth a timestamped line in the UI. Minors surface as the count.
    pub major_faults: Vec<PacifistFault>,
    /// The most frequent Minor-fault criterion this match (criterion + count),
    /// or `None` if there were no Minors. Unlike Majors, individual Minors
    /// aren't itemized (there can be many), but this gives the "Improve" tab
    /// something more specific than a bare count to point at.
    pub top_minor_fault: Option<PacifistFaultCount>,
    pub dimensions: Vec<PacifistDimension>,
}

/// How many times one fault criterion (e.g. `"F9"`) fired at Minor severity.
#[derive(Serialize)]
pub struct PacifistFaultCount {
    pub criterion: String,
    pub count: u32,
}

/// A timestamped fault, labeled by its criteria-spec row (e.g. `"FM-2"`).
#[derive(Serialize)]
pub struct PacifistFault {
    pub t: f32,
    pub criterion: String,
    pub detail: String,
}

/// One dimension row of a player's breakdown.
#[derive(Serialize)]
pub struct PacifistDimension {
    pub label: String,
    pub value: f32,
    pub confidence: f32,
    /// Share of the headline this dimension drove, `0.0..=1.0`.
    pub influence: f32,
    pub evidence_count: usize,
    /// How many opportunities backed this dimension this match.
    pub opportunities: usize,
}

/// Load the rank-relative norms artifact (`rank_norms.json`) from a corpus dir,
/// or `None` if it is absent/unreadable — in which case scoring stays purely
/// absolute (the rank-relative layer is simply omitted).
pub fn load_rank_norms(corpus_dir: &Path) -> Option<RankNorms> {
    let bytes = std::fs::read(corpus_dir.join("rank_norms.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Run the whole pipeline on raw `.replay` bytes, returning the unified bundle.
///
/// `replay_id` labels the match (normally the upload's file stem). Every engine
/// runs at its default config — the same defaults the standalone CLIs use.
///
/// `norms` (optional) enables the **rank-relative** layer: when present, each
/// score is additionally graded against its rank bracket — the lobby's level by
/// default, or `override_bracket` when the caller knows the rank. Passing `None`
/// leaves every report purely absolute (`relative = None`), unchanged.
pub fn analyze(
    bytes: &[u8],
    replay_id: &str,
    norms: Option<&RankNorms>,
    override_bracket: Option<&str>,
) -> Result<Analysis, Box<dyn Error>> {
    // 1. Decode + reconstruct the neutral canonical model — the shared contract.
    let decoded = BoxcarsParser::new().parse(bytes)?;
    let canonical = build_canonical(&decoded, replay_id.to_string());
    let standard_map = is_standard_geometry(canonical.map.as_deref());
    let coordinate_warnings = coordinate_report(&canonical)
        .issues(standard_map)
        .iter()
        .map(CoordinateIssue::message)
        .collect();

    // 2. Decision-discipline scoring (per player) + the lobby report HTML.
    let score_cfg = ScoreConfig::default();
    let mut scores = score_all(&canonical, &score_cfg);
    // Additive rank-relative layer: grade each player against their bracket. No
    // norms ⇒ untouched (every report stays purely absolute).
    if let Some(norms) = norms {
        attach_relative(&mut scores, norms, &score_cfg, override_bracket);
    }
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

    // Core scoreboard stats per player — header truth joined to the track's pri by
    // name, plus a recomputed shooting %. Sorted team then pri (matches bc_stats).
    let mut core: Vec<CorePlayerStats> = canonical
        .tracks
        .iter()
        .map(|t| {
            let m = canonical.players.iter().find(|p| p.name == t.player);
            let (goals, assists, saves, shots, score) = m
                .map(|p| (p.goals, p.assists, p.saves, p.shots, p.score))
                .unwrap_or((0, 0, 0, 0, 0));
            CorePlayerStats {
                pri: t.pri,
                player: t.player.clone(),
                team: t.team,
                goals,
                assists,
                saves,
                shots,
                score,
                shooting_pct: if shots > 0 {
                    goals as f32 / shots as f32 * 100.0
                } else {
                    0.0
                },
            }
        })
        .collect();
    core.sort_by(|a, b| a.team.cmp(&b.team).then(a.pri.cmp(&b.pri)));

    // 6. Pacifist system-adherence scores — bridge the canonical model into the
    //    pacifist domain (touch-decoded possession, shots, touches) and run the
    //    eight-dimension rubric + FM-1 severity for every rostered player.
    let pacifist = {
        let timeline = pacifist_bridge::timeline_from_canonical(&canonical);
        let analyzer = PacifistAnalyzer::default();
        let spans = pacifist_bridge::possession_spans(&canonical);
        let pctx =
            PacifistContext::derive_with_possession(&timeline, analyzer.context_config(), &spans)
                .with_shots(pacifist_bridge::shots(&canonical))
                .with_touches(pacifist_bridge::touches(&canonical));
        let mut players: Vec<PacifistPlayer> = replay_pacifist::roster(&timeline)
            .iter()
            .map(|entry| {
                let score = analyzer.score_player_in(&pctx, entry.player);
                let name = entry.name.clone().unwrap_or_else(|| "—".into());
                PacifistPlayer {
                    platform_id: platform_id_for(&canonical, &name),
                    player: name,
                    team: match entry.team {
                        replay_pacifist::Team::Blue => 0,
                        replay_pacifist::Team::Orange => 1,
                    },
                    value: score.value.map(|v| v.get()),
                    confidence: score.confidence.get(),
                    verdict: match score.faults.verdict {
                        Verdict::Pass => "PASS".into(),
                        Verdict::Fail => "FAIL".into(),
                    },
                    minors: score.faults.minors,
                    majors: score.faults.majors,
                    major_faults: score
                        .faults
                        .faults
                        .iter()
                        .filter(|f| f.severity == Severity::Major)
                        .map(|f| PacifistFault {
                            t: f.t,
                            criterion: f.criterion.to_string(),
                            detail: f.detail.clone(),
                        })
                        .collect(),
                    top_minor_fault: top_minor_fault(&score.faults.faults),
                    dimensions: score
                        .breakdown
                        .iter()
                        .map(|d| PacifistDimension {
                            label: d.dimension.label().to_string(),
                            value: d.value.get(),
                            confidence: d.confidence.get(),
                            influence: d.influence,
                            evidence_count: d.evidence.len(),
                            opportunities: d.opportunities,
                        })
                        .collect(),
                }
            })
            .collect();
        players.sort_by(|a, b| a.team.cmp(&b.team).then_with(|| a.player.cmp(&b.player)));
        PacifistSummary {
            config_version: replay_pacifist::PACIFIST_CONFIG_VERSION.to_string(),
            players,
        }
    };

    // 7. 3D scene with every overlay (roles / win-prob / impact / player stats),
    //    serialized into a self-contained offline viewer document.
    let mut scene = build_scene(&canonical, &skill_report.instances);
    attach_roles(&mut scene, &canonical, &score_cfg);
    attach_winprob(&mut scene, &canonical, &value_cfg);
    attach_impact(&mut scene, &canonical, &value_cfg);
    attach_player_stats(&mut scene, &canonical);
    let viewer_html = html_offline(&scene);

    // 6. Ballchasing-parity dashboard (same canonical model, bc-clone renderer).
    let ballchasing_html =
        bc_clone::html::render_html(&bc_clone::ballchasing_document(&canonical), &canonical);

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
        coordinate_warnings,
        scores,
        skill_profiles,
        impact,
        bc_stats,
        core,
        pacifist,
        viewer_html,
        scoring_html,
        ballchasing_html,
    })
}

/// The platform id of the player called `name`, provided exactly one player in
/// the match has that name (two identical display names would make the mapping a
/// guess, and a wrong id is worse than none).
fn platform_id_for(m: &replay_analyzer::CanonicalMatch, name: &str) -> Option<String> {
    let mut named = m.players.iter().filter(|p| p.name == name);
    match (named.next(), named.next()) {
        (Some(p), None) => p.platform_id.clone(),
        _ => None,
    }
}

/// The most frequent Minor-severity fault criterion in `faults`, or `None` if
/// there were no Minors (ties broken by whichever criterion was seen first —
/// there are only a handful of implemented criteria, so this rarely matters).
fn top_minor_fault(faults: &[replay_pacifist::severity::Fault]) -> Option<PacifistFaultCount> {
    let mut counts: Vec<(&str, u32)> = Vec::new();
    for f in faults.iter().filter(|f| f.severity == Severity::Minor) {
        match counts.iter_mut().find(|(c, _)| *c == f.criterion) {
            Some((_, n)) => *n += 1,
            None => counts.push((f.criterion, 1)),
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(criterion, count)| PacifistFaultCount {
            criterion: criterion.to_string(),
            count,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_pacifist::severity::Fault;

    fn fault(t: f32, severity: Severity, criterion: &'static str) -> Fault {
        Fault {
            t,
            severity,
            criterion,
            detail: String::new(),
        }
    }

    #[test]
    fn top_minor_fault_is_none_without_minors() {
        assert!(top_minor_fault(&[]).is_none());
        let majors_only = [fault(1.0, Severity::Major, "FM-2")];
        assert!(top_minor_fault(&majors_only).is_none());
    }

    #[test]
    fn top_minor_fault_picks_the_most_frequent_criterion() {
        let faults = [
            fault(1.0, Severity::Minor, "F9"),
            fault(2.0, Severity::Minor, "F4"),
            fault(3.0, Severity::Minor, "F9"),
            fault(4.0, Severity::Major, "FM-2"), // majors don't count toward this
            fault(5.0, Severity::Minor, "F9"),
        ];
        let top = top_minor_fault(&faults).expect("has minors");
        assert_eq!(top.criterion, "F9");
        assert_eq!(top.count, 3);
    }
}

//! Golden-file + reconstruction-validation tests on a real sample replay.
//!
//! `42f2.replay` is the validated 2v2 first-cut sample. These tests assert the
//! two validations the first cut passed (ball stays inside the arena; a kickoff
//! frame shows the ball centered with cars on canonical spawns) and pin a
//! compact, deterministic digest of the canonical model as a golden file.
//!
//! Regenerate the golden after an intentional model change:
//! `UPDATE_GOLDEN=1 cargo test --test golden`.

use replay_analyzer::analyze::{self, reconstruct};
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_analyzer::field;
use replay_analyzer::model::{CanonicalMatch, Event};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::LazyLock;

const SAMPLE: &str = "42f2";

/// Parse + analyze the sample replay exactly once for all tests in this binary.
static MATCH: LazyLock<CanonicalMatch> = LazyLock::new(|| {
    let path = sample_path(SAMPLE);
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let decoded = BoxcarsParser::new()
        .parse(&data)
        .unwrap_or_else(|e| panic!("decode {SAMPLE}: {e}"));
    analyze::build_canonical(&decoded, SAMPLE)
});

fn sample_path(id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets/replays")
        .join(format!("{id}.replay"))
}

#[test]
fn ball_stays_inside_arena() {
    let m = &*MATCH;
    let (min, max) = reconstruct::ball_bounds(&m.frames).expect("ball observed");

    // Every ball position must be inside a generous arena envelope. A broken
    // decode produces wild/non-finite coordinates, which this rejects.
    for f in &m.frames {
        if let Some(b) = &f.ball {
            assert!(
                field::ball_in_arena(b.to_arr(), 50.0),
                "ball outside arena at t={}: {:?}",
                f.t,
                b
            );
        }
    }

    // ...and the ball must actually traverse the field (not a frozen artifact):
    // wide horizontal spread and meaningful airborne travel confirm real data.
    assert!(
        max[0] - min[0] > 4000.0,
        "ball x spread too small: {min:?}..{max:?}"
    );
    assert!(
        max[1] - min[1] > 6000.0,
        "ball y spread too small: {min:?}..{max:?}"
    );
    assert!(
        max[2] > 300.0,
        "ball never went meaningfully airborne: zmax={}",
        max[2]
    );
}

#[test]
fn kickoff_frame_has_centered_ball_and_spawn_geometry() {
    let m = &*MATCH;

    // A kickoff: ball resting at field center (~93 uu high) with every live car
    // seated on a canonical spawn at ground level. Mid-play frames where the
    // ball merely passes near center fail the all-cars-on-spawns requirement,
    // so this isolates a genuine kickoff.
    let kickoff = m.frames.iter().find(|f| {
        let Some(b) = &f.ball else { return false };
        b.x.abs() < 6.0
            && b.y.abs() < 6.0
            && (85.0..100.0).contains(&b.z)
            && f.cars.len() >= 2
            && f.cars
                .iter()
                .all(|c| c.p.z < 60.0 && field::is_kickoff_spawn(c.p.to_arr(), 60.0))
    });

    let frame = kickoff.expect("a kickoff frame with centered ball and cars on spawns");
    let ball = frame.ball.as_ref().unwrap();
    assert!(
        (ball.z - field::BALL_RADIUS).abs() < 10.0,
        "kickoff ball rest height: {}",
        ball.z
    );
    assert!(frame.cars.len() >= 2, "kickoff should have >= 2 cars");
}

#[test]
fn coalesced_tracks_are_identity_stable() {
    let m = &*MATCH;
    // Header lists the players that finished with stats; every one of them must
    // appear as exactly one coalesced track (not many actor-id fragments), and
    // every track must coalesce multiple recycled car segments.
    assert!(!m.tracks.is_empty(), "expected coalesced tracks");
    for t in &m.tracks {
        // PRIs are unique per track.
        assert_eq!(
            m.tracks.iter().filter(|o| o.pri == t.pri).count(),
            1,
            "duplicate track for pri {}",
            t.pri
        );
        // This match has demos/respawns, so real players fragment and coalesce.
        assert!(
            t.num_segments >= 1 && t.samples.len() > 100,
            "track {} looks degenerate: {} segments, {} samples",
            t.player,
            t.num_segments,
            t.samples.len()
        );
        // Gaps never contain samples (no carry-forward across a dead window).
        for g in &t.gaps {
            assert!(
                !t.samples.iter().any(|s| s.t > g.start && s.t < g.end),
                "track {} has a sample inside gap [{}, {}]",
                t.player,
                g.start,
                g.end
            );
        }
    }
}

#[test]
fn derived_goal_events_match_header_truth() {
    let m = &*MATCH;
    use std::collections::HashMap;

    // Goal events come from the authoritative header; cross-check their per-
    // scorer counts against the independent PlayerStats goal totals.
    let mut by_scorer: HashMap<&str, i32> = HashMap::new();
    let mut by_team: BTreeMap<i32, i32> = BTreeMap::new();
    for e in &m.events {
        if let Event::Goal { scorer, team, .. } = e {
            if let Some(s) = scorer {
                *by_scorer.entry(s.as_str()).or_default() += 1;
            }
            if let Some(t) = team {
                *by_team.entry(*t).or_default() += 1;
            }
        }
    }

    for p in &m.players {
        if p.goals > 0 {
            assert_eq!(
                by_scorer.get(p.name.as_str()).copied().unwrap_or(0),
                p.goals,
                "goal-event count for {} should match PlayerStats",
                p.name
            );
        }
    }
    // And team goal tallies must equal the header team scores.
    assert_eq!(by_team, m.team_scores, "team goal tallies vs team scores");
}

#[test]
fn derived_features_are_internally_consistent() {
    let m = &*MATCH;
    let dur = m.duration_s;
    assert!(!m.features.is_empty(), "expected per-player features");

    // Independent re-derivation: feature touch counts must sum to the number of
    // touch events (two code paths, same total).
    let touch_total: usize = m.features.iter().map(|f| f.touches).sum();
    let touch_events = m
        .events
        .iter()
        .filter(|e| matches!(e, Event::Touch { .. }))
        .count();
    assert_eq!(
        touch_total, touch_events,
        "feature touches re-derive touch events"
    );

    for f in &m.features {
        assert!(
            (0.0..=dur + 1.0).contains(&f.time_supersonic_s),
            "{} supersonic {} out of [0,{dur}]",
            f.player,
            f.time_supersonic_s
        );
        assert!(
            (0.0..=dur + 1.0).contains(&f.possession_time_s),
            "{} possession",
            f.player
        );
        assert!(f.boost_used >= 0.0, "{} boost_used negative", f.player);
        assert!(
            f.mean_dist_to_ball > 0.0 && f.mean_dist_to_ball < 12_000.0,
            "{} mean_dist {} implausible",
            f.player,
            f.mean_dist_to_ball
        );
    }
}

// ---- Golden digest ----------------------------------------------------------

/// Compact, deterministic fingerprint of the canonical model. Floats are
/// quantized to integers/centi-seconds so the golden is stable across float
/// formatting, while still catching reconstruction regressions.
#[derive(Serialize)]
struct Digest {
    replay_id: String,
    parser_version: String,
    map: Option<String>,
    team_size: Option<i32>,
    record_fps: Option<f32>,
    num_frames: usize,
    duration_cs: i64,
    team_scores: BTreeMap<i32, i32>,
    ball_min: [i32; 3],
    ball_max: [i32; 3],
    tracks: Vec<TrackDigest>,
    /// Event counts by type, plus the goal list (authoritative, stable).
    events_by_type: BTreeMap<String, usize>,
    goals: Vec<GoalDigest>,
    features: Vec<FeatureDigest>,
}

#[derive(Serialize)]
struct FeatureDigest {
    player: String,
    team: Option<i32>,
    touches: usize,
    boost_used: i64,
    supersonic_cs: i64,
    mean_dist: i64,
    possession_cs: i64,
}

#[derive(Serialize)]
struct GoalDigest {
    t_cs: i64,
    scorer: Option<String>,
    team: Option<i32>,
}

#[derive(Serialize)]
struct TrackDigest {
    player: String,
    pri: i32,
    team: Option<i32>,
    num_segments: usize,
    num_samples: usize,
    num_gaps: usize,
    first_cs: i64,
    last_cs: i64,
    p_min: [i32; 3],
    p_max: [i32; 3],
}

fn cs(t: f32) -> i64 {
    (t * 100.0).round() as i64
}

fn digest(m: &CanonicalMatch) -> Digest {
    let (bmin, bmax) = reconstruct::ball_bounds(&m.frames).expect("ball observed");
    let tracks = m
        .tracks
        .iter()
        .map(|t| {
            let mut p_min = [i32::MAX; 3];
            let mut p_max = [i32::MIN; 3];
            for s in &t.samples {
                let p = s.p.to_arr();
                for i in 0..3 {
                    p_min[i] = p_min[i].min(p[i].round() as i32);
                    p_max[i] = p_max[i].max(p[i].round() as i32);
                }
            }
            TrackDigest {
                player: t.player.clone(),
                pri: t.pri,
                team: t.team,
                num_segments: t.num_segments,
                num_samples: t.samples.len(),
                num_gaps: t.gaps.len(),
                first_cs: t.samples.first().map(|s| cs(s.t)).unwrap_or(0),
                last_cs: t.samples.last().map(|s| cs(s.t)).unwrap_or(0),
                p_min,
                p_max,
            }
        })
        .collect();

    let mut events_by_type: BTreeMap<String, usize> = BTreeMap::new();
    let mut goals = Vec::new();
    for e in &m.events {
        let key = match e {
            Event::Kickoff { .. } => "kickoff",
            Event::Touch { .. } => "touch",
            Event::Possession { .. } => "possession",
            Event::Demo { .. } => "demo",
            Event::Goal { t, scorer, team } => {
                goals.push(GoalDigest {
                    t_cs: cs(*t),
                    scorer: scorer.clone(),
                    team: *team,
                });
                "goal"
            }
            Event::Stat { .. } => "stat",
        };
        *events_by_type.entry(key.to_string()).or_default() += 1;
    }
    let features = m
        .features
        .iter()
        .map(|f| FeatureDigest {
            player: f.player.clone(),
            team: f.team,
            touches: f.touches,
            boost_used: f.boost_used.round() as i64,
            supersonic_cs: cs(f.time_supersonic_s),
            mean_dist: f.mean_dist_to_ball.round() as i64,
            possession_cs: cs(f.possession_time_s),
        })
        .collect();

    Digest {
        replay_id: m.replay_id.clone(),
        parser_version: m.parser_version.clone(),
        map: m.map.clone(),
        team_size: m.team_size,
        record_fps: m.record_fps,
        num_frames: m.num_frames,
        duration_cs: cs(m.duration_s),
        team_scores: m.team_scores.clone(),
        ball_min: bmin.map(|v| v.round() as i32),
        ball_max: bmax.map(|v| v.round() as i32),
        tracks,
        events_by_type,
        goals,
        features,
    }
}

#[test]
fn canonical_model_matches_golden() {
    let actual = serde_json::to_string_pretty(&digest(&MATCH)).unwrap();
    let golden_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{SAMPLE}.json"));

    if std::env::var_os("UPDATE_GOLDEN").is_some() || !golden_path.exists() {
        std::fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
        std::fs::write(&golden_path, &actual).unwrap();
        eprintln!("wrote golden {}", golden_path.display());
        return;
    }

    let expected = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("read golden {}: {e}", golden_path.display()));
    // Normalize line endings: a CRLF-converting checkout (git autocrlf) must not
    // diverge from serde's LF output.
    let norm = |s: &str| s.replace("\r\n", "\n").trim().to_string();
    assert_eq!(
        norm(&actual),
        norm(&expected),
        "canonical digest drifted from golden; if intended, rerun with UPDATE_GOLDEN=1"
    );
}

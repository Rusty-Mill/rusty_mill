//! Lobby-completeness → confidence: a player who leaves or goes AFK for a real
//! stretch must drop confidence for the *whole* lobby (§11), not just their own
//! report — mirrors `map_class.rs`'s non-standard-map test.

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_analyzer::model::Vec3;
use replay_scoring::{score_all, Confidence, ScoreConfig};
use std::path::PathBuf;

fn decode_sample() -> replay_analyzer::model::CanonicalMatch {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rleval/assets/replays/42f2.replay");
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read sample: {e}"));
    let decoded = BoxcarsParser::new().parse(&data).expect("decode");
    build_canonical(&decoded, "42f2")
}

#[test]
fn full_lobby_sample_is_unaffected() {
    let canonical = decode_sample();
    let cfg = ScoreConfig::default();
    let reports = score_all(&canonical, &cfg);
    assert!(!reports.is_empty());
    for r in &reports {
        // "Unlucky Odd" is a lone player on a 1-man team in this sample (2v1 by
        // roster, not 2v2) — their `second_man` sub-score is already `None` for
        // a pre-existing, unrelated reason (no partner ever fills the 2nd-man
        // role), independent of this test's coverage gate. Confirmed by
        // stashing the coverage change and re-running: same result.
        if r.target_player == "Unlucky Odd" {
            continue;
        }
        assert_eq!(
            r.confidence,
            Confidence::Ok,
            "{} unexpectedly low-confidence on the clean sample",
            r.target_player
        );
    }
}

#[test]
fn early_leaver_drops_confidence_for_the_whole_lobby() {
    let mut canonical = decode_sample();
    // Truncate one track to its first quarter — simulates a disconnect partway
    // through, well past `min_span_frac` of the whole match.
    let cutoff = canonical.duration_s * 0.25;
    let leaver = canonical.tracks[0].pri;
    canonical.tracks[0].samples.retain(|s| s.t <= cutoff);

    let cfg = ScoreConfig::default();
    let reports = score_all(&canonical, &cfg);
    assert_eq!(reports.len(), canonical.tracks.len());
    for r in &reports {
        assert_eq!(
            r.confidence,
            Confidence::LowConfidence,
            "{} (pri {}) should be low-confidence once pri {leaver} leaves early",
            r.target_player,
            r.target_pri
        );
    }
}

#[test]
fn mid_game_afk_drops_confidence_even_without_leaving() {
    let mut canonical = decode_sample();
    // Freeze one track's velocity for a long mid-game stretch without
    // shortening its span — it never "leaves", it just stops playing.
    let t0 = canonical.duration_s * 0.4;
    let t1 = canonical.duration_s * 0.6;
    for s in canonical.tracks[1]
        .samples
        .iter_mut()
        .filter(|s| s.t >= t0 && s.t <= t1)
    {
        s.v = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
    }

    let cfg = ScoreConfig::default();
    let reports = score_all(&canonical, &cfg);
    for r in &reports {
        assert_eq!(
            r.confidence,
            Confidence::LowConfidence,
            "{} should be low-confidence once a lobby-mate goes AFK mid-game",
            r.target_player
        );
    }
}

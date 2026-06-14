//! Analyze layer: turn a neutral [`DecodedReplay`] into the [`CanonicalMatch`].
//!
//! This layer is decoder-agnostic — it depends only on the [`crate::decode`]
//! port types — and scoring-agnostic. It is the pure core that golden-file and
//! fixture tests exercise without ever touching a `.replay` file.

pub mod identity;
pub mod normalize;
pub mod reconstruct;
pub mod resample;

use crate::decode::DecodedReplay;
use crate::model::{CanonicalMatch, Resampled};

/// Default resample grid rate (Hz). Matches RL's nominal record rate.
pub const DEFAULT_HZ: f32 = 30.0;

/// This analyzer crate's version, pinned into the canonical model alongside the
/// decoder version for reproducibility.
pub const ANALYZER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build the canonical match model from a decoded replay.
///
/// `replay_id` is supplied by the caller (typically the file stem or a content
/// hash) since the neutral decode carries no notion of where bytes came from.
pub fn build_canonical(decoded: &DecodedReplay, replay_id: impl Into<String>) -> CanonicalMatch {
    let recon = reconstruct::reconstruct(decoded);
    let duration_s = recon.frames.last().map(|f| f.t).unwrap_or(0.0);

    // T2: derive attack-direction signs, then resample to the fixed grid. Both
    // borrow `recon`, so compute before moving its fields into the model.
    let team_attack_sign = normalize::team_attack_sign(&recon.frames, &recon.tracks);
    let grid = resample::resample(&recon, DEFAULT_HZ);
    let resampled = Resampled {
        hz: DEFAULT_HZ,
        team_attack_sign,
        frames: grid,
    };

    CanonicalMatch {
        replay_id: replay_id.into(),
        parser_version: decoded.meta.parser_version.clone(),
        analyzer_version: ANALYZER_VERSION.to_string(),
        map: decoded.meta.map.clone(),
        team_size: decoded.meta.team_size,
        record_fps: decoded.meta.record_fps,
        num_frames: recon.frames.len(),
        duration_s,
        team_scores: decoded.meta.team_scores.clone(),
        players: decoded.meta.players.clone(),
        tracks: recon.tracks,
        frames: recon.frames,
        resampled,
    }
}

//! Analyze layer: turn a neutral [`DecodedReplay`] into the [`CanonicalMatch`].
//!
//! This layer is decoder-agnostic — it depends only on the [`crate::decode`]
//! port types — and scoring-agnostic. It is the pure core that golden-file and
//! fixture tests exercise without ever touching a `.replay` file.

pub mod identity;
pub mod reconstruct;

use crate::decode::DecodedReplay;
use crate::model::CanonicalMatch;

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
    }
}

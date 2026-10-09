//! Artifact bytes and their typed payloads. The payload the core validates
//! must derive from the exact bytes stored (plan amendment A1): the driver
//! decodes the stored bytes with [`decode_artifact`] and refuses a command
//! whose claimed payload differs.
//!
//! On-disk shapes: `brief`, `diff` and `log` are raw bytes with a unit
//! payload; `candidate` and `test_report` are their JSON; `spec` is JSON with
//! `brief` (the brief's artifact id) and `markdown`.

use crate::ids::ArtId;
use crate::record::*;
use rusty_serde::{Deserialize, Serialize};

/// The stored form of a `spec` artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecFile {
    pub brief: ArtId,
    pub markdown: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DecodeError {}

fn json<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, DecodeError> {
    let text = std::str::from_utf8(bytes).map_err(|e| DecodeError(e.to_string()))?;
    rusty_serde::json::from_str(text).map_err(|e| DecodeError(e.to_string()))
}

/// Decode the typed payload a stored artifact of `kind` carries.
pub fn decode_artifact(kind: ArtifactKind, bytes: &[u8]) -> Result<ArtifactPayload, DecodeError> {
    Ok(match kind {
        ArtifactKind::Brief => ArtifactPayload::Brief,
        ArtifactKind::Diff => ArtifactPayload::Diff,
        ArtifactKind::Log => ArtifactPayload::Log,
        ArtifactKind::Spec => ArtifactPayload::Spec {
            brief: json::<SpecFile>(bytes)?.brief,
        },
        ArtifactKind::Candidate => ArtifactPayload::Candidate(json(bytes)?),
        ArtifactKind::TestReport => ArtifactPayload::TestReport(json(bytes)?),
    })
}

/// Produce the bytes whose decoding yields `payload`. `body` is the raw
/// content for `brief`, `diff` and `log`, and the Markdown for `spec`;
/// it is ignored for `candidate` and `test_report`.
pub fn encode_artifact(payload: &ArtifactPayload, body: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let text = match payload {
        ArtifactPayload::Brief | ArtifactPayload::Diff | ArtifactPayload::Log => {
            return Ok(body.to_vec())
        }
        ArtifactPayload::Spec { brief } => rusty_serde::json::to_string(&SpecFile {
            brief: *brief,
            markdown: String::from_utf8_lossy(body).into_owned(),
        }),
        ArtifactPayload::Candidate(c) => rusty_serde::json::to_string(c),
        ArtifactPayload::TestReport(r) => rusty_serde::json::to_string(r),
    };
    text.map(String::into_bytes)
        .map_err(|e| DecodeError(e.to_string()))
}

//! Artifact bytes and their typed payloads. The payload the core validates
//! must derive from the exact bytes stored (plan amendment A1): the driver
//! decodes the stored bytes with [`decode_artifact`] and refuses a command
//! whose claimed payload differs.
//!
//! On-disk shapes: `brief`, `diff` and `log` are raw bytes with a unit
//! payload; `candidate` and `test_report` are their JSON; `spec` is JSON with
//! `brief` (the brief's artifact id) and either `markdown` (a snapshot) or
//! `edit` (a co-edited version: `base` and `ops`, see [`crate::coedit`]).

use crate::coedit::{LineOp, SpecEdit};
use crate::ids::ArtId;
use crate::record::*;
use rusty_serde::{Deserialize, Serialize};

/// The stored form of a `spec` artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecFile {
    pub brief: ArtId,
    pub markdown: String,
}

/// Either stored form of a `spec`: a snapshot's `markdown` or a version's `edit`.
#[derive(Serialize, Deserialize)]
struct SpecBytes {
    brief: ArtId,
    #[rusty_serde(default, skip_serializing_if = "Option::is_none")]
    markdown: Option<String>,
    #[rusty_serde(default, skip_serializing_if = "Option::is_none")]
    edit: Option<EditBytes>,
}

#[derive(Serialize, Deserialize)]
struct EditBytes {
    #[rusty_serde(default)]
    base: Option<ArtId>,
    ops: Vec<LineOp>,
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
        ArtifactKind::Spec => spec_payload(json(bytes)?)?,
        ArtifactKind::Candidate => ArtifactPayload::Candidate(json(bytes)?),
        ArtifactKind::TestReport => ArtifactPayload::TestReport(json(bytes)?),
    })
}

fn spec_payload(s: SpecBytes) -> Result<ArtifactPayload, DecodeError> {
    match (s.markdown, s.edit) {
        (Some(_), None) => Ok(ArtifactPayload::Spec { brief: s.brief }),
        (None, Some(e)) => Ok(ArtifactPayload::SpecEdit(SpecEdit {
            brief: s.brief,
            base: e.base,
            ops: e.ops,
        })),
        _ => Err(DecodeError(
            "a spec holds either `markdown` or an `edit`".into(),
        )),
    }
}

/// The Markdown of a stored snapshot `spec` artifact. A co-edited version
/// stores its edit, not its text: read that from [`crate::TaskState::doc`].
pub fn spec_markdown(bytes: &[u8]) -> Result<String, DecodeError> {
    Ok(json::<SpecFile>(bytes)?.markdown)
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
        ArtifactPayload::SpecEdit(e) => rusty_serde::json::to_string(&SpecBytes {
            brief: e.brief,
            markdown: None,
            edit: Some(EditBytes {
                base: e.base,
                ops: e.ops.clone(),
            }),
        }),
        ArtifactPayload::Candidate(c) => rusty_serde::json::to_string(c),
        ArtifactPayload::TestReport(r) => rusty_serde::json::to_string(r),
    };
    text.map(String::into_bytes)
        .map_err(|e| DecodeError(e.to_string()))
}

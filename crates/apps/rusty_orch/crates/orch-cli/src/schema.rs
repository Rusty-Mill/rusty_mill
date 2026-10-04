//! The JSON Schema of a reply, for CLIs that constrain their final message
//! to one: `codex exec --output-schema` and `claude -p --json-schema`.
//!
//! Codex forwards it as a strict Structured Outputs schema, which imposes
//! rules beyond plain JSON Schema: every object must list every property
//! in `required` and set `additionalProperties: false`, and optional
//! fields do not exist. The schema is written to that stricter form so one
//! text serves every consumer. Kind-dependent fields (`confidence` for findings,
//! `verdict` for reviews) are therefore expressed as one `anyOf` variant
//! per kind, each fully required. Size caps are not in the schema; the
//! parser enforces them, so the schema never has to guess which keywords a
//! given provider accepts.
//!
//! [`crate::parse`] remains the single authority: it decides which kinds a
//! role may write and rejects anything the schema let through.
//! Under [`StopRule::BestEffort`] the `question` variant is left out, so a
//! model constrained by the schema cannot even form one (ADR-0011).

use orch_core::goal::StopRule;

const HEAD: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["entries"],
  "properties": {
    "entries": {
      "type": "array",
      "items": {
        "anyOf": [
"#;

const TAIL: &str = r#"
        ]
      }
    }
  }
}
"#;

const FINDING: &str = r#"          {
            "type": "object",
            "additionalProperties": false,
            "required": ["kind", "confidence", "body", "refs"],
            "properties": {
              "kind": { "type": "string", "enum": ["finding"] },
              "confidence": { "type": "string", "enum": ["low", "medium", "high"] },
              "body": { "type": "string" },
              "refs": { "type": "array", "items": { "type": "string" } }
            }
          }"#;

const QUESTION: &str = r#"          {
            "type": "object",
            "additionalProperties": false,
            "required": ["kind", "body", "refs"],
            "properties": {
              "kind": { "type": "string", "enum": ["question"] },
              "body": { "type": "string" },
              "refs": { "type": "array", "items": { "type": "string" } }
            }
          }"#;

const ASSUMPTION: &str = r#"          {
            "type": "object",
            "additionalProperties": false,
            "required": ["kind", "body", "refs"],
            "properties": {
              "kind": { "type": "string", "enum": ["assumption"] },
              "body": { "type": "string" },
              "refs": { "type": "array", "items": { "type": "string" } }
            }
          }"#;

const REVIEW: &str = r#"          {
            "type": "object",
            "additionalProperties": false,
            "required": ["kind", "verdict", "body", "refs"],
            "properties": {
              "kind": { "type": "string", "enum": ["review"] },
              "verdict": { "type": "string", "enum": ["approve", "changes_requested"] },
              "body": { "type": "string" },
              "refs": { "type": "array", "items": { "type": "string" } }
            }
          }"#;

/// The schema text for a run under `stop`. Strictness is checked by
/// `orch-codex`'s contract test, the consumer that needs it.
pub fn output_schema(stop: StopRule) -> String {
    let variants: &[&str] = match stop {
        StopRule::Checkpoint => &[FINDING, QUESTION, ASSUMPTION, REVIEW],
        StopRule::BestEffort => &[FINDING, ASSUMPTION, REVIEW],
    };
    format!("{HEAD}{}{TAIL}", variants.join(",\n"))
}

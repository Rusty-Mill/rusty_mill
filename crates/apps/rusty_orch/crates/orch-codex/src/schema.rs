//! The JSON Schema handed to `codex exec --output-schema`.
//!
//! Codex forwards it as a strict Structured Outputs schema, which imposes
//! rules beyond plain JSON Schema: every object must list every property
//! in `required` and set `additionalProperties: false`, and optional
//! fields do not exist. Kind-dependent fields (`confidence` for findings,
//! `verdict` for reviews) are therefore expressed as one `anyOf` variant
//! per kind, each fully required. Size caps are not in the schema; the
//! parser enforces them, so the schema never has to guess which keywords a
//! given provider accepts.
//!
//! [`orch_cli::parse`] remains the single authority: it decides which
//! kinds a role may write and rejects anything the schema let through.

/// The schema text, written to a scratch file per run. Strictness is
/// checked by `tests/contract.rs`.
pub const OUTPUT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["entries"],
  "properties": {
    "entries": {
      "type": "array",
      "items": {
        "anyOf": [
          {
            "type": "object",
            "additionalProperties": false,
            "required": ["kind", "confidence", "body", "refs"],
            "properties": {
              "kind": { "type": "string", "enum": ["finding"] },
              "confidence": { "type": "string", "enum": ["low", "medium", "high"] },
              "body": { "type": "string" },
              "refs": { "type": "array", "items": { "type": "string" } }
            }
          },
          {
            "type": "object",
            "additionalProperties": false,
            "required": ["kind", "body", "refs"],
            "properties": {
              "kind": { "type": "string", "enum": ["question"] },
              "body": { "type": "string" },
              "refs": { "type": "array", "items": { "type": "string" } }
            }
          },
          {
            "type": "object",
            "additionalProperties": false,
            "required": ["kind", "body", "refs"],
            "properties": {
              "kind": { "type": "string", "enum": ["assumption"] },
              "body": { "type": "string" },
              "refs": { "type": "array", "items": { "type": "string" } }
            }
          },
          {
            "type": "object",
            "additionalProperties": false,
            "required": ["kind", "verdict", "body", "refs"],
            "properties": {
              "kind": { "type": "string", "enum": ["review"] },
              "verdict": { "type": "string", "enum": ["approve", "changes_requested"] },
              "body": { "type": "string" },
              "refs": { "type": "array", "items": { "type": "string" } }
            }
          }
        ]
      }
    }
  }
}
"#;

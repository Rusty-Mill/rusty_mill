//! The JSON Schema handed to `codex exec --output-schema`.
//!
//! It mirrors what [`orch_cli::parse`] accepts, so the model is constrained
//! to the protocol before the parser sees the reply. Kinds, confidence and
//! verdict are enumerated; the parser still decides which kinds a role may
//! write and whether confidence or verdict is required.

/// The schema text, written to a scratch file per run.
pub const OUTPUT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["entries"],
  "properties": {
    "entries": {
      "type": "array",
      "minItems": 1,
      "maxItems": 8,
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["kind", "body", "refs"],
        "properties": {
          "kind": { "type": "string", "enum": ["finding", "question", "assumption", "review"] },
          "confidence": { "type": "string", "enum": ["low", "medium", "high"] },
          "verdict": { "type": "string", "enum": ["approve", "changes_requested"] },
          "body": { "type": "string", "maxLength": 500 },
          "refs": { "type": "array", "items": { "type": "string" } }
        }
      }
    }
  }
}
"#;

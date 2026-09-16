//! Secret redaction by default (ADR-0026). A deny-list of argument keys plus a
//! value scrub, run before any tool args/results are journaled or emitted. It
//! scrubs *values, not structure*, so verification/attribution traces stay
//! intact. The authoritative deny-list lives in `docs/architecture/threat-model.md`;
//! this is the v1 default.

use serde_json::{Map, Value};

/// Replacement placeholder for a redacted value.
pub const REDACTED: &str = "[redacted]";

/// Argument-key substrings whose value is always scrubbed (case-insensitive).
const DENY_KEY_SUBSTRINGS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "authorization",
    "credential",
    "private_key",
    "access_key",
];

/// Token prefixes that mark a value as a secret regardless of its key.
const SECRET_PREFIXES: &[&str] = &["sk-", "ghp_", "gho_", "github_pat_", "xoxb-", "AKIA"];

fn key_is_secret(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    DENY_KEY_SUBSTRINGS.iter().any(|d| lower.contains(d))
}

fn token_is_secret(tok: &str) -> bool {
    if SECRET_PREFIXES.iter().any(|p| tok.starts_with(p)) {
        return true;
    }
    // Long, unbroken, high-entropy-looking run (base64/hex-ish) → treat as secret.
    tok.len() >= 40
        && tok.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == '-' || c == '_'
        })
}

/// Scrub secret-looking tokens out of free text. Detection and scrubbing both
/// split on `char::is_whitespace` (`split_whitespace`) — splitting only on
/// literal ASCII space (as an earlier version did) let a tab/newline-delimited
/// secret pass the detection check but survive the scrub unredacted. Original
/// whitespace formatting is not preserved; tokens are rejoined with a single
/// space.
pub fn redact_text(s: &str) -> String {
    if !s.split_whitespace().any(token_is_secret) {
        return s.to_string();
    }
    s.split_whitespace()
        .map(|tok| if token_is_secret(tok) { REDACTED } else { tok })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Recursively redact a JSON value: deny-listed object keys have their value
/// replaced wholesale; string values are token-scrubbed; structure is preserved.
pub fn redact_value(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, val) in map {
                if key_is_secret(k) {
                    out.insert(k.clone(), Value::String(REDACTED.to_string()));
                } else {
                    out.insert(k.clone(), redact_value(val));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        Value::String(s) => Value::String(redact_text(s)),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scrubs_denylisted_keys_keeps_structure() {
        let v = json!({"path": "src/a.rs", "api_key": "abc123", "nested": {"token": "xyz"}});
        let r = redact_value(&v);
        assert_eq!(r["path"], json!("src/a.rs"));
        assert_eq!(r["api_key"], json!(REDACTED));
        assert_eq!(r["nested"]["token"], json!(REDACTED));
    }

    #[test]
    fn scrubs_secret_looking_values() {
        assert_eq!(
            redact_text("use key sk-ABCDEFGHIJKLMNOP now"),
            "use key [redacted] now"
        );
        assert_eq!(redact_text("a normal sentence"), "a normal sentence");
    }

    #[test]
    fn scrubs_secrets_delimited_by_non_space_whitespace() {
        // No literal ASCII space anywhere: a tab/newline-delimited secret must
        // still be caught. Pre-fix, `split(' ')` scrubbed nothing here (it saw
        // one unbroken "token"), so the secret survived unredacted even though
        // `split_whitespace()`-based detection correctly flagged it.
        let secret = format!("sk-{}", "a".repeat(40));
        let input = format!("prefix\n{secret}\tsuffix");
        let out = redact_text(&input);
        assert!(!out.contains(&secret), "secret leaked into: {out}");
        assert!(out.contains(REDACTED));
    }
}

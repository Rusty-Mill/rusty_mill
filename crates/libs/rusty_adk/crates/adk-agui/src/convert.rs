//! Values cross the boundary in both directions: ADK speaks `serde_json`,
//! AG-UI speaks `rusty_json`.

use rusty_agui::Error;
use serde_json::Value as Adk;

/// A `serde_json` value as a `rusty_json` one.
pub fn to_agui(value: &Adk) -> Result<rusty_json::Value, Error> {
    rusty_json::to_value(value).map_err(|error| Error::Agent(format!("value: {error}")))
}

/// A `rusty_json` value as a `serde_json` one.
pub fn to_adk(value: &rusty_json::Value) -> Result<Adk, Error> {
    rusty_json::from_value(value.clone()).map_err(|error| Error::Agent(format!("value: {error}")))
}

/// The person's answer to a suspended node: JSON when it parses, text otherwise.
///
/// A rendered form that answers with structured data sends JSON; a plain
/// "approved" is a string either way.
pub fn resume_payload(text: &str) -> Adk {
    serde_json::from_str(text).unwrap_or_else(|_| Adk::String(text.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn values_round_trip() {
        let adk = json!({"a": [1, 2.5, "x", null, true], "b": {"c": {}}});
        let agui = to_agui(&adk).unwrap();
        assert_eq!(to_adk(&agui).unwrap(), adk);
    }

    #[test]
    fn a_resume_payload_is_json_when_it_parses() {
        assert_eq!(resume_payload("{\"ok\": true}"), json!({"ok": true}));
        assert_eq!(resume_payload("approved"), json!("approved"));
        assert_eq!(resume_payload("42"), json!(42));
    }
}

//! The chat-completions body in both directions. Pure: no I/O.

use rusty_json::{Map, Value};

/// One question: a stable, cacheable system part and the per-call part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prompt<'a> {
    /// Sent as the `system` message. Identical across calls, so the
    /// server can reuse its cached prefix.
    pub system: &'a str,
    /// Sent as the `user` message.
    pub user: &'a str,
    pub max_output_tokens: u32,
}

/// Token counts as the server reported them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    /// `None` when the server did not report it as an integer.
    pub input_tokens: Option<u64>,
    /// `None` when the server did not report it as an integer.
    pub output_tokens: Option<u64>,
    /// Prompt tokens served from the server's cache, when it says.
    pub cached_input_tokens: Option<u64>,
}

/// The model's reply text, not yet interpreted, and what it cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub text: String,
    pub usage: Usage,
}

/// The JSON request body for `POST {base}/chat/completions`.
pub fn request_body(model: &str, prompt: &Prompt<'_>, json_mode: bool) -> String {
    let mut body = Map::new();
    body.insert("model".into(), model.into());
    body.insert(
        "messages".into(),
        Value::Array(vec![
            message("system", prompt.system),
            message("user", prompt.user),
        ]),
    );
    body.insert("max_tokens".into(), prompt.max_output_tokens.into());
    body.insert("stream".into(), false.into());
    if json_mode {
        let mut format = Map::new();
        format.insert("type".into(), "json_object".into());
        body.insert("response_format".into(), format.into());
    }
    Value::Object(body).to_json_string()
}

fn message(role: &str, content: &str) -> Value {
    let mut m = Map::new();
    m.insert("role".into(), role.into());
    m.insert("content".into(), content.into());
    m.into()
}

/// The first choice's text and the usage block. A count the server omits
/// stays unknown (`None`), never zero; a missing reply is an error.
pub fn parse_response(raw: &str) -> Result<Completion, String> {
    let v = Value::parse(raw).map_err(|e| format!("response is not JSON: {e}"))?;
    let text = v
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or("response has no choices[0].message.content")?;
    let count = |p: &str| v.pointer(p).and_then(Value::as_u64);
    Ok(Completion {
        text: text.to_owned(),
        usage: Usage {
            input_tokens: count("/usage/prompt_tokens"),
            output_tokens: count("/usage/completion_tokens"),
            cached_input_tokens: count("/usage/prompt_tokens_details/cached_tokens"),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROMPT: Prompt<'static> = Prompt {
        system: "rules \"quoted\"",
        user: "the claim",
        max_output_tokens: 512,
    };

    #[test]
    fn request_carries_system_then_user_and_the_limits() {
        let v = Value::parse(&request_body("qwen", &PROMPT, true)).expect("json");
        assert_eq!(v.pointer("/model").and_then(Value::as_str), Some("qwen"));
        assert_eq!(
            v.pointer("/messages/0/role").and_then(Value::as_str),
            Some("system")
        );
        assert_eq!(
            v.pointer("/messages/0/content").and_then(Value::as_str),
            Some("rules \"quoted\"")
        );
        assert_eq!(
            v.pointer("/messages/1/content").and_then(Value::as_str),
            Some("the claim")
        );
        assert_eq!(v.pointer("/max_tokens").and_then(Value::as_u64), Some(512));
        assert_eq!(
            v.pointer("/response_format/type").and_then(Value::as_str),
            Some("json_object")
        );
    }

    #[test]
    fn json_mode_off_sends_no_response_format() {
        let v = Value::parse(&request_body("m", &PROMPT, false)).expect("json");
        assert!(v.get("response_format").is_none());
    }

    #[test]
    fn response_text_and_usage() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"{\"posts\":[]}"}}],
            "usage":{"prompt_tokens":120,"completion_tokens":9,"prompt_tokens_details":{"cached_tokens":100}}}"#;
        let c = parse_response(raw).expect("parses");
        assert_eq!(c.text, r#"{"posts":[]}"#);
        assert_eq!(
            c.usage,
            Usage {
                input_tokens: Some(120),
                output_tokens: Some(9),
                cached_input_tokens: Some(100)
            }
        );
    }

    #[test]
    fn missing_usage_is_unknown_and_missing_content_is_refused() {
        let c = parse_response(r#"{"choices":[{"message":{"content":"x"}}]}"#).expect("parses");
        assert_eq!(c.usage, Usage::default());
        let c = parse_response(
            r#"{"choices":[{"message":{"content":"x"}}],"usage":{"prompt_tokens":"9","completion_tokens":-1}}"#,
        )
        .expect("parses");
        assert_eq!((c.usage.input_tokens, c.usage.output_tokens), (None, None));
        assert!(parse_response(r#"{"choices":[]}"#).is_err());
        assert!(parse_response("not json").is_err());
    }
}

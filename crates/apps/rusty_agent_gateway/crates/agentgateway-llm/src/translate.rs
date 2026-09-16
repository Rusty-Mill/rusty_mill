//! Translating between the OpenAI chat-completions wire format and the shapes
//! other providers speak.
//!
//! Anthropic's Messages API is here; Gemini's `generateContent` is in
//! [`gemini`], because it differs in more than field names — the model is in
//! the URL rather than the body, and streaming is a different method.
//!
//! # Why only a translated provider needs types
//!
//! For an OpenAI-compatible provider the gateway forwards the request body
//! essentially unchanged, so it works on the raw JSON and never builds a typed
//! model of it. That is deliberate: a typed round-trip would silently drop
//! every field this crate has not heard of — tool definitions, `response_
//! format`, `logprobs`, whatever OpenAI ships next — and a gateway that
//! quietly deletes half a request is worse than one that refuses it.
//!
//! Translation is the only place a typed view is unavoidable, and even there
//! the unknown fields are dropped *visibly*: the shapes genuinely differ, so
//! there is no honest passthrough to preserve.
//!
//! # Three differences that bite
//!
//! - **`max_tokens` is optional for OpenAI and required by Anthropic.** A
//!   request that omits it is perfectly valid on one side and rejected on the
//!   other, so translation has to supply a default rather than pass the
//!   absence through.
//! - **The system prompt is a message for OpenAI and a top-level field for
//!   Anthropic.** Left in the message list it would either be rejected or,
//!   worse, silently treated as a user turn.
//! - **Finish reasons use different vocabularies.** `end_turn` means what
//!   OpenAI calls `stop`, and a client switching providers should not have to
//!   learn both.

pub mod gemini;

use serde_json::{Map, Value, json};

use crate::tools;

/// Anthropic rejects a request without `max_tokens`, so one has to be chosen
/// when the caller did not.
///
/// Deliberately generous: too low silently truncates an answer, which reads as
/// a model problem rather than a gateway default, and is far harder to
/// diagnose than a bill.
pub const DEFAULT_MAX_TOKENS: u64 = 4096;

/// A request this gateway cannot translate.
#[derive(Debug, thiserror::Error)]
pub enum TranslateError {
    /// The body was not a JSON object.
    #[error("the request body is not a JSON object")]
    NotAnObject,

    /// `messages` was missing or not an array.
    #[error("`messages` is missing or not an array")]
    Messages,

    /// The caller named a model that cannot go in a URL path.
    #[error(
        "`{model}` is not a usable model name: Gemini names the model in the request path, so \
         it may hold only letters, digits, dots, dashes and underscores"
    )]
    ModelName {
        /// What the caller asked for.
        model: String,
    },
}

/// Token counts, however the provider spelled them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// Tokens in the prompt.
    pub prompt: u64,
    /// Tokens generated.
    pub completion: u64,
}

impl Usage {
    /// Total tokens billed.
    pub fn total(self) -> u64 {
        self.prompt.saturating_add(self.completion)
    }
}

/// The model a request asks for.
pub fn requested_model(body: &Value) -> Option<&str> {
    body.get("model").and_then(Value::as_str)
}

/// Whether the caller asked for a streamed response.
pub fn is_streaming(body: &Value) -> bool {
    body.get("stream").and_then(Value::as_bool).unwrap_or(false)
}

/// Force the model, leaving everything else untouched.
pub fn set_model(body: &mut Value, model: &str) {
    if let Some(object) = body.as_object_mut() {
        object.insert("model".into(), Value::String(model.to_string()));
    }
}

/// Convert an OpenAI chat-completions request into an Anthropic Messages one.
pub fn to_anthropic(body: &Value) -> Result<Value, TranslateError> {
    let object = body.as_object().ok_or(TranslateError::NotAnObject)?;
    let messages = object
        .get("messages")
        .and_then(Value::as_array)
        .ok_or(TranslateError::Messages)?;

    // System prompts are a top-level field for Anthropic, not a turn. Several
    // are concatenated rather than dropped: a caller that split its
    // instructions across two system messages meant both of them.
    let mut system = Vec::new();
    let mut turns = Vec::new();

    for message in messages {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        let content = message.get("content").cloned().unwrap_or(Value::Null);

        match role {
            "system" | "developer" => {
                if let Some(text) = content.as_str() {
                    system.push(text.to_string());
                }
            }
            "assistant" => {
                // An assistant turn that called a tool becomes a list of
                // content blocks rather than a string: for Anthropic the call
                // lives *in* the content, beside whatever the model said.
                let content = tools::assistant_to_anthropic(message).unwrap_or(content);
                turns.push(json!({"role": "assistant", "content": content}));
            }
            // Anthropic has no `tool` role: a result is a block inside a user
            // turn. Consecutive results join one turn, because two user
            // messages in a row are rejected -- and a model asked to call three
            // tools answers all three before the conversation moves on.
            "tool" => {
                let block = tools::result_to_anthropic(message);
                match turns.last_mut().filter(|last| is_tool_result_turn(last)) {
                    Some(last) => {
                        if let Some(blocks) = last.get_mut("content").and_then(Value::as_array_mut)
                        {
                            blocks.push(block);
                        }
                    }
                    None => turns.push(json!({"role": "user", "content": [block]})),
                }
            }
            _ => turns.push(json!({"role": "user", "content": content})),
        }
    }

    let mut out = Map::new();
    if let Some(model) = object.get("model") {
        out.insert("model".into(), model.clone());
    }
    out.insert("messages".into(), Value::Array(turns));
    out.insert(
        "max_tokens".into(),
        object
            .get("max_tokens")
            .or_else(|| object.get("max_completion_tokens"))
            .and_then(Value::as_u64)
            .map_or(json!(DEFAULT_MAX_TOKENS), |n| json!(n)),
    );

    if !system.is_empty() {
        out.insert("system".into(), Value::String(system.join("\n\n")));
    }
    // Shared spellings carry straight over.
    for key in ["temperature", "top_p", "stream"] {
        if let Some(value) = object.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }
    // `stop` is `stop_sequences`, and Anthropic wants an array even for one.
    if let Some(stop) = object.get("stop") {
        let sequences = match stop {
            Value::String(one) => json!([one]),
            other => other.clone(),
        };
        out.insert("stop_sequences".into(), sequences);
    }

    // Tools were dropped here entirely until now, which was worse than not
    // supporting them: `finish_reason` already translated `tool_use`, so a
    // client could be told a tool ran and never be told which.
    if let Some(tools) = object
        .get("tools")
        .and_then(tools::definitions_to_anthropic)
    {
        out.insert("tools".into(), tools);
    }
    if let Some(choice) = object
        .get("tool_choice")
        .and_then(tools::choice_to_anthropic)
    {
        out.insert("tool_choice".into(), choice);
    }

    Ok(Value::Object(out))
}

/// Whether this turn is a user message made only of tool results.
///
/// Anthropic rejects two user turns in a row, so consecutive results have to
/// join one — but only when the previous turn *is* one, or a result would be
/// appended to an ordinary question.
fn is_tool_result_turn(turn: &Value) -> bool {
    turn.get("role").and_then(Value::as_str) == Some("user")
        && turn
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                !blocks.is_empty()
                    && blocks.iter().all(|block| {
                        block.get("type").and_then(Value::as_str) == Some("tool_result")
                    })
            })
}

/// Convert an Anthropic Messages response into an OpenAI one.
pub fn from_anthropic(body: &Value, created: u64) -> Value {
    let text = body
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    let usage = anthropic_usage(body).unwrap_or_default();

    let mut message = Map::new();
    message.insert("role".into(), json!("assistant"));
    // `null` rather than `""` when the model only called a tool: OpenAI's own
    // responses do that, and a client checking `content` for emptiness reads
    // the two the same way while one checking for null does not.
    message.insert(
        "content".into(),
        if text.is_empty() {
            Value::Null
        } else {
            json!(text)
        },
    );
    if let Some(calls) = body.get("content").and_then(tools::calls_from_anthropic) {
        message.insert("tool_calls".into(), calls);
    }

    json!({
        "id": body.get("id").and_then(Value::as_str).unwrap_or("chatcmpl-unknown"),
        "object": "chat.completion",
        "created": created,
        "model": body.get("model").and_then(Value::as_str).unwrap_or_default(),
        "choices": [{
            "index": 0,
            "message": Value::Object(message),
            "finish_reason": finish_reason(body.get("stop_reason").and_then(Value::as_str)),
        }],
        "usage": {
            "prompt_tokens": usage.prompt,
            "completion_tokens": usage.completion,
            "total_tokens": usage.total(),
        },
    })
}

/// Map an Anthropic stop reason onto OpenAI's vocabulary.
pub fn finish_reason(stop_reason: Option<&str>) -> Value {
    match stop_reason {
        Some("end_turn") | Some("stop_sequence") => json!("stop"),
        Some("max_tokens") => json!("length"),
        Some("tool_use") => json!("tool_calls"),
        // `null` is what OpenAI sends mid-stream, and an unrecognised reason
        // is better reported as "unknown" than mislabelled as a clean stop.
        None => Value::Null,
        Some(_) => json!("stop"),
    }
}

/// Token usage from an Anthropic response.
pub fn anthropic_usage(body: &Value) -> Option<Usage> {
    let usage = body.get("usage")?;
    Some(Usage {
        prompt: usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        completion: usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

/// Token usage from an OpenAI response.
pub fn openai_usage(body: &Value) -> Option<Usage> {
    let usage = body.get("usage")?;
    Some(Usage {
        prompt: usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        completion: usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Value {
        json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "system", "content": "Be brief."},
                {"role": "user", "content": "Hello"},
            ],
            "temperature": 0.2,
        })
    }

    #[test]
    fn the_system_prompt_becomes_a_top_level_field() {
        // Left in the message list Anthropic either rejects it or treats it as
        // a user turn, which silently changes what the model was told.
        let out = to_anthropic(&request()).expect("should translate");
        assert_eq!(out["system"], "Be brief.");

        let messages = out["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 1, "the system turn is not also a message");
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn several_system_prompts_are_joined_rather_than_dropped() {
        let mut body = request();
        body["messages"] = json!([
            {"role": "system", "content": "First."},
            {"role": "developer", "content": "Second."},
            {"role": "user", "content": "Hi"},
        ]);

        let out = to_anthropic(&body).expect("should translate");
        assert_eq!(
            out["system"], "First.\n\nSecond.",
            "a caller that split its instructions meant both halves"
        );
    }

    #[test]
    fn a_missing_max_tokens_gets_a_default() {
        // Optional for OpenAI, required by Anthropic: passing the absence
        // through would turn a valid request into a rejected one.
        let out = to_anthropic(&request()).expect("should translate");
        assert_eq!(out["max_tokens"], DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn an_explicit_max_tokens_is_respected() {
        let mut body = request();
        body["max_tokens"] = json!(100);
        let out = to_anthropic(&body).expect("should translate");
        assert_eq!(out["max_tokens"], 100);
    }

    #[test]
    fn the_newer_max_completion_tokens_spelling_is_honoured() {
        let mut body = request();
        body["max_completion_tokens"] = json!(50);
        let out = to_anthropic(&body).expect("should translate");
        assert_eq!(out["max_tokens"], 50);
    }

    #[test]
    fn a_string_stop_becomes_an_array() {
        let mut body = request();
        body["stop"] = json!("END");
        let out = to_anthropic(&body).expect("should translate");
        assert_eq!(
            out["stop_sequences"],
            json!(["END"]),
            "Anthropic wants a list even for one sequence"
        );
    }

    #[test]
    fn shared_parameters_carry_over() {
        let out = to_anthropic(&request()).expect("should translate");
        assert_eq!(out["temperature"], 0.2);
        assert_eq!(out["model"], "gpt-4o");
    }

    #[test]
    fn a_body_without_messages_is_refused() {
        let err = to_anthropic(&json!({"model": "x"})).expect_err("should not translate");
        assert!(matches!(err, TranslateError::Messages));
    }

    fn anthropic_response() -> Value {
        json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4",
            "content": [
                {"type": "text", "text": "Hello"},
                {"type": "text", "text": " there"},
            ],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 12, "output_tokens": 5},
        })
    }

    #[test]
    fn a_response_translates_into_openai_shape() {
        let out = from_anthropic(&anthropic_response(), 1_700_000_000);

        assert_eq!(out["object"], "chat.completion");
        assert_eq!(out["id"], "msg_123");
        assert_eq!(out["model"], "claude-sonnet-4");
        assert_eq!(
            out["choices"][0]["message"]["content"], "Hello there",
            "several text blocks are one message to an OpenAI client"
        );
        assert_eq!(out["choices"][0]["message"]["role"], "assistant");
    }

    #[test]
    fn usage_is_renamed_and_totalled() {
        let out = from_anthropic(&anthropic_response(), 0);
        assert_eq!(out["usage"]["prompt_tokens"], 12);
        assert_eq!(out["usage"]["completion_tokens"], 5);
        assert_eq!(
            out["usage"]["total_tokens"], 17,
            "OpenAI clients read the total; Anthropic does not send one"
        );
    }

    #[test]
    fn finish_reasons_are_mapped_to_openai_vocabulary() {
        // A client switching providers should not have to learn both.
        assert_eq!(finish_reason(Some("end_turn")), json!("stop"));
        assert_eq!(finish_reason(Some("stop_sequence")), json!("stop"));
        assert_eq!(finish_reason(Some("max_tokens")), json!("length"));
        assert_eq!(finish_reason(Some("tool_use")), json!("tool_calls"));
        assert_eq!(
            finish_reason(None),
            Value::Null,
            "null is what OpenAI sends mid-stream"
        );
    }

    #[test]
    fn a_non_text_block_does_not_corrupt_the_message() {
        let mut body = anthropic_response();
        body["content"] = json!([
            {"type": "thinking", "thinking": "hidden"},
            {"type": "text", "text": "visible"},
        ]);
        let out = from_anthropic(&body, 0);
        assert_eq!(out["choices"][0]["message"]["content"], "visible");
    }

    #[test]
    fn the_model_can_be_forced() {
        let mut body = request();
        set_model(&mut body, "claude-sonnet-4");
        assert_eq!(requested_model(&body), Some("claude-sonnet-4"));
    }

    #[test]
    fn streaming_is_detected_from_the_body() {
        assert!(!is_streaming(&request()));
        let mut body = request();
        body["stream"] = json!(true);
        assert!(is_streaming(&body));
    }

    #[test]
    fn usage_reads_both_providers_spellings() {
        assert_eq!(
            anthropic_usage(&anthropic_response()),
            Some(Usage {
                prompt: 12,
                completion: 5
            })
        );
        assert_eq!(
            openai_usage(&json!({"usage": {"prompt_tokens": 3, "completion_tokens": 4}})),
            Some(Usage {
                prompt: 3,
                completion: 4
            })
        );
        assert_eq!(openai_usage(&json!({})), None);
    }
}

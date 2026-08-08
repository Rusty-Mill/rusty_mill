//! Translating between OpenAI chat-completions and Gemini's `generateContent`.
//!
//! Gemini differs from the other two providers in more than field names, and
//! the differences are the reason this is a module of its own.
//!
//! # The model is in the URL, not the body
//!
//! OpenAI and Anthropic both name the model in the request body, so the address
//! a route dials is fixed and can be resolved once at startup. Gemini puts it
//! in the path — `/v1beta/models/{model}:generateContent` — which makes the URL
//! a function of what the caller asked for.
//!
//! That is a client-controlled string in a URL the gateway signs with its own
//! key, so [`model_path`] checks it rather than trusting it. A `model` of
//! `../../../v1beta/tunedModels/x` would otherwise point an authenticated call
//! at an endpoint nobody configured.
//!
//! # A turn is `parts`, and there is no system role
//!
//! Content is a list of parts even when it is one string, roles are `user` and
//! `model` rather than `user` and `assistant`, and the system prompt is a
//! top-level `systemInstruction` — the same rearrangement Anthropic needs, in a
//! different shape.
//!
//! # Sampling settings live in `generationConfig`
//!
//! `temperature` and `top_p` are not top-level fields but members of a nested
//! object, and `max_tokens` is `maxOutputTokens`. A translation that carried
//! them over by name would produce a request Gemini accepts and quietly ignores
//! — worse than one it rejects, because nothing says the settings were lost.

use serde_json::{Map, Value, json};

use super::{TranslateError, Usage};

/// Turn a caller's model name into the path segment Gemini expects.
///
/// The prefix is optional in what a caller sends — the API's own examples use
/// both `gemini-2.5-flash` and `models/gemini-2.5-flash` — so one is accepted
/// and normalised rather than being a way to write the same request wrong.
///
/// What is left after the prefix must look like a model name. This is the one
/// place a client's string reaches a URL this gateway authenticates, and a name
/// carrying `/` or `:` would choose a different method or a different resource
/// on the same host.
pub fn model_path(model: &str) -> Result<String, TranslateError> {
    let bare = model.strip_prefix("models/").unwrap_or(model);
    let usable = !bare.is_empty()
        && bare
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));

    match usable {
        true => Ok(format!("models/{bare}")),
        false => Err(TranslateError::ModelName {
            model: model.to_string(),
        }),
    }
}

/// Convert an OpenAI chat-completions request into a `generateContent` one.
///
/// `model` and `stream` are deliberately **not** carried over: the first is in
/// the URL and the second is a different method there, and Gemini rejects a
/// request holding fields it does not know.
pub fn to_gemini(body: &Value) -> Result<Value, TranslateError> {
    let object = body.as_object().ok_or(TranslateError::NotAnObject)?;
    let messages = object
        .get("messages")
        .and_then(Value::as_array)
        .ok_or(TranslateError::Messages)?;

    // Streaming is a different method on a different URL here, not a field, so
    // there is nothing to carry over and nothing that half works. Refused for
    // the same reason tools are: the alternative is a client that asked for a
    // stream, was answered in one piece, and has to work out why.
    if super::is_streaming(body) {
        return Err(TranslateError::Unsupported {
            provider: "gemini",
            what: "streaming",
        });
    }

    // Tools are refused rather than dropped. A request that asks for them and
    // gets an ordinary answer looks like a model that chose not to call one,
    // which is a far more expensive thing to debug than a refusal.
    for key in ["tools", "tool_choice", "functions"] {
        if object.get(key).is_some_and(|value| !value.is_null()) {
            return Err(TranslateError::Unsupported {
                provider: "gemini",
                what: "tool calling",
            });
        }
    }
    if messages
        .iter()
        .any(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
    {
        return Err(TranslateError::Unsupported {
            provider: "gemini",
            what: "tool calling",
        });
    }

    let mut system = Vec::new();
    let mut contents: Vec<Value> = Vec::new();

    for message in messages {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        let content = message.get("content").unwrap_or(&Value::Null);

        if matches!(role, "system" | "developer") {
            if let Some(text) = content.as_str() {
                system.push(text.to_string());
            }
            continue;
        }

        let parts = parts_for(content);
        // A turn with nothing in it is dropped rather than sent: Gemini rejects
        // an empty `parts`, and an assistant turn that only called a tool has
        // already been refused above.
        if parts.is_empty() {
            continue;
        }

        let role = match role {
            "assistant" => "model",
            _ => "user",
        };
        // Gemini rejects two turns of the same role in a row, and a caller that
        // sent two user messages meant both. Joining their parts keeps every
        // word; dropping one would silently change the question.
        match contents
            .last_mut()
            .filter(|last| last.get("role").and_then(Value::as_str) == Some(role))
        {
            Some(last) => {
                if let Some(existing) = last.get_mut("parts").and_then(Value::as_array_mut) {
                    existing.extend(parts);
                }
            }
            None => contents.push(json!({"role": role, "parts": parts})),
        }
    }

    let mut out = Map::new();
    out.insert("contents".into(), Value::Array(contents));

    if !system.is_empty() {
        // No role: an instruction is not a turn, and Gemini rejects one that
        // carries a role here.
        out.insert(
            "systemInstruction".into(),
            json!({"parts": [{"text": system.join("\n\n")}]}),
        );
    }

    let mut config = Map::new();
    for (openai, gemini) in [("temperature", "temperature"), ("top_p", "topP")] {
        if let Some(value) = object.get(openai) {
            config.insert(gemini.to_string(), value.clone());
        }
    }
    if let Some(max) = object
        .get("max_tokens")
        .or_else(|| object.get("max_completion_tokens"))
        .and_then(Value::as_u64)
    {
        config.insert("maxOutputTokens".into(), json!(max));
    }
    // `stop` is `stopSequences`, and Gemini wants a list even for one.
    if let Some(stop) = object.get("stop") {
        let sequences = match stop {
            Value::String(one) => json!([one]),
            other => other.clone(),
        };
        config.insert("stopSequences".into(), sequences);
    }
    // Absent rather than empty: `generationConfig: {}` is accepted, but an
    // empty object on every request is noise in anyone's provider-side logs.
    if !config.is_empty() {
        out.insert("generationConfig".into(), Value::Object(config));
    }

    Ok(Value::Object(out))
}

/// Every part of one OpenAI message, in the shape Gemini reads.
///
/// The multimodal list form is carried across for its text parts. An image part
/// is dropped: Gemini takes bytes as `inline_data` or a `file_data` URI and
/// never a URL to fetch, so there is no honest translation of an
/// `image_url` — and inventing one would send a request that fails at the
/// provider with a message about a field the caller never wrote.
fn parts_for(content: &Value) -> Vec<Value> {
    match content {
        Value::String(text) if !text.is_empty() => vec![json!({"text": text})],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .filter(|text| !text.is_empty())
            .map(|text| json!({"text": text}))
            .collect(),
        _ => Vec::new(),
    }
}

/// Convert a `generateContent` response into an OpenAI one.
pub fn from_gemini(body: &Value, created: u64) -> Value {
    let candidate = body
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first());

    let text = candidate.map(text_of).unwrap_or_default();
    let usage = usage(body).unwrap_or_default();

    json!({
        "id": body.get("responseId").and_then(Value::as_str).unwrap_or("chatcmpl-unknown"),
        "object": "chat.completion",
        "created": created,
        "model": body.get("modelVersion").and_then(Value::as_str).unwrap_or_default(),
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                // `null` rather than `""` for an answer with no text, matching
                // what OpenAI itself sends: a client checking for null reads
                // the two differently.
                "content": if text.is_empty() { Value::Null } else { json!(text) },
            },
            "finish_reason": finish_reason(
                candidate.and_then(|c| c.get("finishReason")).and_then(Value::as_str),
            ),
        }],
        "usage": {
            "prompt_tokens": usage.prompt,
            "completion_tokens": usage.completion,
            "total_tokens": usage.total(),
        },
    })
}

/// The text of one candidate, with its parts joined.
pub fn text_of(candidate: &Value) -> String {
    candidate
        .get("content")
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// Map a Gemini finish reason onto OpenAI's vocabulary.
///
/// The refusal reasons — `SAFETY`, `BLOCKLIST`, `PROHIBITED_CONTENT`,
/// `RECITATION` — have no OpenAI equivalent at all, and reporting them as a
/// clean `stop` would tell a client the model finished its thought when it was
/// cut off. OpenAI's own vocabulary has `content_filter` for exactly this, so
/// that is where they go.
pub fn finish_reason(reason: Option<&str>) -> Value {
    match reason {
        Some("STOP") => json!("stop"),
        Some("MAX_TOKENS") => json!("length"),
        Some("SAFETY")
        | Some("BLOCKLIST")
        | Some("PROHIBITED_CONTENT")
        | Some("SPII")
        | Some("RECITATION") => json!("content_filter"),
        // `null` mid-stream, which is what OpenAI sends before the last chunk.
        None => Value::Null,
        Some(_) => json!("stop"),
    }
}

/// Token usage from a Gemini response.
///
/// `candidatesTokenCount` is absent when the answer was refused before any
/// tokens were generated, and reading that as zero is right: nothing was
/// generated. The prompt was still read and still billed.
pub fn usage(body: &Value) -> Option<Usage> {
    let usage = body.get("usageMetadata")?;
    Some(Usage {
        prompt: usage
            .get("promptTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        completion: usage
            .get("candidatesTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

#[cfg(test)]
mod tests;

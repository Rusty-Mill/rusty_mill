//! Tool calling across the OpenAI and Anthropic shapes.
//!
//! `to_anthropic` used to drop `tools` on the floor, and the effect was worse
//! than a missing feature. [`crate::translate::finish_reason`] already maps
//! Anthropic's `tool_use` onto OpenAI's `tool_calls`, so a client that somehow
//! got a tool call back was handed `finish_reason: "tool_calls"` with no calls
//! attached — a response that says a tool ran and cannot say which.
//!
//! # Four translations, not one
//!
//! A tool call is a round trip, and every leg of it is spelled differently:
//!
//! 1. **Definitions.** OpenAI wraps each in `{type: "function", function:
//!    {...}}` and calls the schema `parameters`; Anthropic takes the function
//!    flat and calls it `input_schema`.
//! 2. **Choice.** OpenAI's `auto` / `none` / `required` / a named function
//!    become Anthropic's `{type: auto}` / `{type: none}` / `{type: any}` /
//!    `{type: tool, name}`. `required` and `any` mean the same thing under
//!    different words.
//! 3. **The call, going back out.** An assistant turn carries `tool_calls`
//!    beside its text for OpenAI, and `tool_use` *content blocks* for
//!    Anthropic — so the message content changes shape, not just its fields.
//! 4. **The result, coming back in.** OpenAI sends a whole message with
//!    `role: "tool"`; Anthropic sends a `tool_result` block inside a *user*
//!    turn. Consecutive results belong in one turn, because Anthropic rejects
//!    two user messages in a row.
//!
//! # Arguments are a string on one side and an object on the other
//!
//! OpenAI's `function.arguments` is a JSON string; Anthropic's `input` is a
//! JSON object. Translating either way means parsing or serializing, and both
//! can fail on input a model produced. A partial or malformed argument string
//! becomes an empty object rather than an error: the alternative is failing a
//! whole conversation over one call the model garbled, which the client could
//! otherwise see and retry.

use serde_json::{Map, Value, json};

/// Translate OpenAI tool definitions into Anthropic's shape.
///
/// Returns `None` when there is nothing to translate, so the caller can leave
/// the field off entirely — Anthropic treats an empty `tools` array as a
/// request to use no tools, which is not the same as not mentioning them.
pub fn definitions_to_anthropic(tools: &Value) -> Option<Value> {
    let tools = tools.as_array()?;
    let translated: Vec<Value> = tools
        .iter()
        .filter_map(|tool| {
            // OpenAI has only ever had `type: "function"` here, but a newer
            // kind should be skipped rather than sent as one.
            match tool.get("type").and_then(Value::as_str) {
                Some("function") | None => {}
                Some(other) => {
                    tracing::debug!(
                        kind = other,
                        "dropping a tool kind Anthropic has no shape for"
                    );
                    return None;
                }
            }
            let function = tool.get("function")?;
            let name = function.get("name")?.as_str()?;

            let mut out = Map::new();
            out.insert("name".into(), json!(name));
            if let Some(description) = function.get("description") {
                out.insert("description".into(), description.clone());
            }
            // `parameters` is `input_schema`, and Anthropic requires one. A
            // tool with no parameters still has a schema: the empty object.
            out.insert(
                "input_schema".into(),
                function
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
            );
            Some(Value::Object(out))
        })
        .collect();

    (!translated.is_empty()).then_some(Value::Array(translated))
}

/// Translate OpenAI's `tool_choice` into Anthropic's.
pub fn choice_to_anthropic(choice: &Value) -> Option<Value> {
    match choice {
        Value::String(word) => match word.as_str() {
            "auto" => Some(json!({"type": "auto"})),
            "none" => Some(json!({"type": "none"})),
            // Different words for the same instruction: pick one, any one.
            "required" | "any" => Some(json!({"type": "any"})),
            _ => None,
        },
        Value::Object(object) => {
            let name = object
                .get("function")
                .and_then(|f| f.get("name"))
                .or_else(|| object.get("name"))?
                .as_str()?;
            Some(json!({"type": "tool", "name": name}))
        }
        _ => None,
    }
}

/// Turn an OpenAI assistant turn into Anthropic content blocks.
///
/// `None` when the turn has no tool calls, so the caller keeps the content it
/// already had rather than rebuilding an identical thing.
pub fn assistant_to_anthropic(message: &Value) -> Option<Value> {
    let calls = message.get("tool_calls")?.as_array()?;
    if calls.is_empty() {
        return None;
    }

    let mut blocks = Vec::new();
    // Text first: Anthropic puts the model's own words and its tool calls in
    // one block list, in the order it produced them, and a call with no
    // preceding text is normal.
    if let Some(text) = message.get("content").and_then(Value::as_str)
        && !text.is_empty()
    {
        blocks.push(json!({"type": "text", "text": text}));
    }

    for call in calls {
        let Some(function) = call.get("function") else {
            continue;
        };
        let Some(name) = function.get("name").and_then(Value::as_str) else {
            continue;
        };
        blocks.push(json!({
            "type": "tool_use",
            "id": call.get("id").and_then(Value::as_str).unwrap_or_default(),
            "name": name,
            "input": arguments_to_input(function.get("arguments")),
        }));
    }

    (!blocks.is_empty()).then_some(Value::Array(blocks))
}

/// Turn an OpenAI `role: "tool"` message into an Anthropic `tool_result` block.
pub fn result_to_anthropic(message: &Value) -> Value {
    let content = match message.get("content") {
        // Anthropic takes the result as text; a structured one is serialized
        // rather than dropped, since the model reads it either way.
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    json!({
        "type": "tool_result",
        "tool_use_id": message
            .get("tool_call_id")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "content": content,
    })
}

/// Collect the tool calls in an Anthropic response into OpenAI's shape.
///
/// `None` when the response made none, so the caller sends no `tool_calls`
/// field at all — an empty array is a claim that the model considered tools
/// and declined, which is not what happened.
pub fn calls_from_anthropic(content: &Value) -> Option<Value> {
    let blocks = content.as_array()?;
    let calls: Vec<Value> = blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .map(|block| {
            json!({
                "id": block.get("id").and_then(Value::as_str).unwrap_or_default(),
                "type": "function",
                "function": {
                    "name": block.get("name").and_then(Value::as_str).unwrap_or_default(),
                    "arguments": input_to_arguments(block.get("input")),
                },
            })
        })
        .collect();

    (!calls.is_empty()).then_some(Value::Array(calls))
}

/// OpenAI's argument string, as an Anthropic input object.
///
/// A malformed or partial string becomes an empty object rather than an error:
/// failing a whole conversation over one call a model garbled is worse than
/// forwarding a call the model can be told went wrong.
fn arguments_to_input(arguments: Option<&Value>) -> Value {
    match arguments {
        Some(Value::String(text)) => serde_json::from_str(text).unwrap_or_else(|_| json!({})),
        Some(Value::Object(object)) => Value::Object(object.clone()),
        _ => json!({}),
    }
}

/// An Anthropic input object, as OpenAI's argument string.
fn input_to_arguments(input: Option<&Value>) -> String {
    match input {
        Some(value) => serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()),
        None => "{}".to_string(),
    }
}

#[cfg(test)]
mod tests;

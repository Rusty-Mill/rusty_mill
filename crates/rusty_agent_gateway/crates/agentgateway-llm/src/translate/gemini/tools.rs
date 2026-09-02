//! Tool calling in Gemini's shape.
//!
//! The same four translations [`crate::tools`] describes for Anthropic, spelled
//! a third way — and with two problems that side does not have.
//!
//! # A result is matched by name, not by id
//!
//! OpenAI's tool result carries a `tool_call_id` and no name. Anthropic's
//! `tool_result` carries the same id, so that leg is a rename. Gemini's
//! `functionResponse` carries the **function's name** and the call it answers
//! carries no id at all, so the id has to be resolved to a name — which is only
//! possible by looking back at the assistant turn that made the call.
//!
//! So [`Conversation`] walks the messages in order, remembering each call's id
//! and name as it goes. A result whose id was never announced is dropped rather
//! than sent under a guessed name: a `functionResponse` naming the wrong
//! function is answered as though the wrong tool ran.
//!
//! Calls this gateway produced carry ids it made up — Gemini does not send one
//! — so the ids in a conversation are the ones handed out by [`calls_from`],
//! and a client that echoes back what it was given round-trips exactly.
//!
//! # A parameter schema has to be cut down
//!
//! Gemini's `Schema` is a subset of JSON Schema, and its JSON parser **rejects
//! fields it does not know** rather than ignoring them. An OpenAI tool
//! definition written for strict mode carries `additionalProperties: false`,
//! which is enough on its own to fail the whole request with `Unknown name
//! "additionalProperties"`.
//!
//! So [`schema_for`] keeps the fields Gemini defines and drops the rest. The
//! allow-list errs towards the known-good set, because the two failures are not
//! symmetric: a dropped constraint loosens validation the model was going to
//! treat as advice anyway, while a kept unknown field refuses the call outright.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

/// The fields Gemini's `Schema` accepts.
///
/// Everything else — `additionalProperties`, `$schema`, `$ref`, `oneOf`,
/// `allOf`, `const` — is dropped. See the module docs for why the list is a
/// allow-list rather than a deny-list.
const SCHEMA_FIELDS: &[&str] = &[
    "type",
    "format",
    "title",
    "description",
    "nullable",
    "enum",
    "items",
    "properties",
    "required",
    "propertyOrdering",
    "anyOf",
    "default",
    "example",
    "minimum",
    "maximum",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "minProperties",
    "maxProperties",
    "pattern",
];

/// Translate OpenAI tool definitions into Gemini's `tools`.
///
/// One `functionDeclarations` list rather than one entry per tool: Gemini's
/// `tools` is a list of *kinds* of tool — function declarations, search
/// retrieval, code execution — not a list of functions.
///
/// `None` when there is nothing to translate, so the caller leaves the field
/// off. An empty list is a statement about tools rather than the absence of
/// one.
pub fn definitions_for(tools: &Value) -> Option<Value> {
    let declarations: Vec<Value> = tools
        .as_array()?
        .iter()
        .filter_map(|tool| {
            // OpenAI has only ever had `type: "function"` here, but a newer
            // kind should be skipped rather than sent as one.
            match tool.get("type").and_then(Value::as_str) {
                Some("function") | None => {}
                Some(other) => {
                    tracing::debug!(kind = other, "dropping a tool kind Gemini has no shape for");
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
            // A function with no parameters sends none: Gemini takes the field
            // as optional, and an empty object schema is accepted but says
            // something slightly different from saying nothing.
            if let Some(parameters) = function.get("parameters") {
                out.insert("parameters".into(), schema_for(parameters));
            }
            Some(Value::Object(out))
        })
        .collect();

    (!declarations.is_empty()).then(|| json!([{"functionDeclarations": declarations}]))
}

/// A JSON Schema cut down to what Gemini's `Schema` accepts.
///
/// Recursive, because the fields it rejects appear at every level — a nested
/// object's `additionalProperties` fails the request exactly as the outer one
/// does.
pub fn schema_for(schema: &Value) -> Value {
    let Some(object) = schema.as_object() else {
        return schema.clone();
    };

    let mut out = Map::new();
    for (key, value) in object {
        if !SCHEMA_FIELDS.contains(&key.as_str()) {
            tracing::debug!(field = %key, "dropping a schema field Gemini does not accept");
            continue;
        }
        let value = match key.as_str() {
            "properties" => match value.as_object() {
                Some(properties) => Value::Object(
                    properties
                        .iter()
                        .map(|(name, nested)| (name.clone(), schema_for(nested)))
                        .collect(),
                ),
                None => value.clone(),
            },
            "items" => schema_for(value),
            "anyOf" => match value.as_array() {
                Some(branches) => Value::Array(branches.iter().map(schema_for).collect()),
                None => value.clone(),
            },
            _ => value.clone(),
        };
        out.insert(key.clone(), value);
    }
    Value::Object(out)
}

/// Translate OpenAI's `tool_choice` into Gemini's `toolConfig`.
pub fn choice_for(choice: &Value) -> Option<Value> {
    let config = match choice {
        Value::String(word) => match word.as_str() {
            "auto" => json!({"mode": "AUTO"}),
            "none" => json!({"mode": "NONE"}),
            // Different words for the same instruction: call something.
            "required" | "any" => json!({"mode": "ANY"}),
            _ => return None,
        },
        Value::Object(object) => {
            let name = object
                .get("function")
                .and_then(|function| function.get("name"))
                .or_else(|| object.get("name"))?
                .as_str()?;
            // Naming one function is `ANY` narrowed to it. Gemini has no
            // single-tool mode of its own.
            json!({"mode": "ANY", "allowedFunctionNames": [name]})
        }
        _ => return None,
    };
    Some(json!({"functionCallingConfig": config}))
}

/// The calls a conversation has announced, by id.
///
/// Built as the messages are walked, so a result is always matched against
/// calls that came before it. See the module docs.
#[derive(Debug, Default)]
pub struct Conversation {
    names: BTreeMap<String, String>,
}

impl Conversation {
    /// Remember the calls in an assistant turn, and return them as parts.
    ///
    /// `None` when the turn made no calls, so the caller keeps the text parts
    /// it already built.
    pub fn calls_in(&mut self, message: &Value) -> Option<Vec<Value>> {
        let calls = message.get("tool_calls")?.as_array()?;
        if calls.is_empty() {
            return None;
        }

        let mut parts = Vec::new();
        for call in calls {
            let Some(function) = call.get("function") else {
                continue;
            };
            let Some(name) = function.get("name").and_then(Value::as_str) else {
                continue;
            };
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                self.names.insert(id.to_string(), name.to_string());
            }
            parts.push(json!({"functionCall": {
                "name": name,
                "args": arguments_to_args(function.get("arguments")),
            }}));
        }

        (!parts.is_empty()).then_some(parts)
    }

    /// Turn an OpenAI `role: "tool"` message into a `functionResponse` part.
    ///
    /// `None` when the id was never announced. Sending it under a guessed name
    /// would have the model read one tool's output as another's, which is worse
    /// than the call appearing unanswered.
    pub fn result_in(&self, message: &Value) -> Option<Value> {
        let name = match message.get("tool_call_id").and_then(Value::as_str) {
            Some(id) => self.names.get(id).cloned(),
            // The deprecated `role: "function"` shape carried the name itself,
            // and some clients still send it beside `tool_call_id`.
            None => None,
        }
        .or_else(|| {
            message
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })?;

        Some(json!({"functionResponse": {"name": name, "response": response_for(message)}}))
    }
}

/// A tool result as the object Gemini requires.
///
/// `response` is a structured object there and a string for OpenAI. One that
/// parses as an object is used as it stands, since that is what the tool
/// actually returned; anything else is wrapped, because a bare string is not a
/// valid `response` and dropping it would answer the call with nothing.
fn response_for(message: &Value) -> Value {
    match message.get("content") {
        Some(Value::String(text)) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(object)) => Value::Object(object),
            _ => json!({"output": text}),
        },
        Some(Value::Object(object)) => Value::Object(object.clone()),
        Some(other) => json!({"output": other}),
        None => json!({}),
    }
}

/// Collect the calls in a Gemini candidate into OpenAI's `tool_calls`.
///
/// The ids are made up: Gemini's `functionCall` carries none, and an OpenAI
/// client needs one to send the result back under. They are positional, which
/// is enough — they only have to be unique within the response that hands them
/// out, and that is the only place they are ever compared.
///
/// `None` when the candidate called nothing, so the caller sends no field at
/// all. An empty array claims the model considered tools and declined.
pub fn calls_from(candidate: &Value) -> Option<Value> {
    let parts = candidate
        .get("content")
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)?;

    let calls: Vec<Value> = parts
        .iter()
        .filter_map(|part| part.get("functionCall"))
        .enumerate()
        .map(|(index, call)| call_at(index as u64, call))
        .collect();

    (!calls.is_empty()).then_some(Value::Array(calls))
}

/// One `functionCall` as an OpenAI tool call, numbered from the response.
pub fn call_at(index: u64, call: &Value) -> Value {
    json!({
        "id": call_id(index),
        "type": "function",
        "function": {
            "name": call.get("name").and_then(Value::as_str).unwrap_or_default(),
            "arguments": args_to_arguments(call.get("args")),
        },
    })
}

/// The id handed out for the call at this position.
pub fn call_id(index: u64) -> String {
    format!("call_{index}")
}

/// OpenAI's argument string, as Gemini's `args` object.
///
/// A malformed or partial string becomes an empty object rather than an error,
/// for the reason [`crate::tools`] gives: failing a whole conversation over one
/// call a model garbled is worse than forwarding a call it can be told went
/// wrong.
fn arguments_to_args(arguments: Option<&Value>) -> Value {
    match arguments {
        Some(Value::String(text)) => serde_json::from_str(text).unwrap_or_else(|_| json!({})),
        Some(Value::Object(object)) => Value::Object(object.clone()),
        _ => json!({}),
    }
}

/// Gemini's `args` object, as OpenAI's argument string.
fn args_to_arguments(args: Option<&Value>) -> String {
    match args {
        Some(value) => serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()),
        None => "{}".to_string(),
    }
}

#[cfg(test)]
mod tests;

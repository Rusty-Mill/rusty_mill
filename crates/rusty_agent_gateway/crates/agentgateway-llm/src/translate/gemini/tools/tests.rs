//! Unit tests for Gemini's tool shapes.

use serde_json::json;

use super::*;

fn definitions() -> Value {
    json!([{
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Look up the weather",
            "parameters": {
                "type": "object",
                "properties": {"city": {"type": "string", "description": "Which city"}},
                "required": ["city"],
            },
        },
    }])
}

#[test]
fn definitions_go_into_one_function_declarations_list() {
    // Gemini's `tools` is a list of kinds of tool, not a list of functions.
    let out = definitions_for(&definitions()).expect("should translate");
    let list = out.as_array().expect("a list of tool kinds");
    assert_eq!(list.len(), 1);

    let declarations = list[0]["functionDeclarations"]
        .as_array()
        .expect("declarations");
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0]["name"], "get_weather");
    assert_eq!(declarations[0]["description"], "Look up the weather");
    assert_eq!(declarations[0]["parameters"]["required"], json!(["city"]));
}

#[test]
fn two_functions_share_the_one_declarations_list() {
    let mut tools = definitions();
    tools.as_array_mut().expect("array").push(json!({
        "type": "function",
        "function": {"name": "get_time", "parameters": {"type": "object"}},
    }));

    let out = definitions_for(&tools).expect("should translate");
    assert_eq!(out.as_array().expect("array").len(), 1);
    assert_eq!(
        out[0]["functionDeclarations"]
            .as_array()
            .expect("declarations")
            .len(),
        2
    );
}

#[test]
fn nothing_to_translate_leaves_the_field_off() {
    // An empty list is a statement about tools rather than the absence of one.
    assert!(definitions_for(&json!([])).is_none());
    assert!(definitions_for(&json!("not a list")).is_none());
    assert!(definitions_for(&json!([{"type": "retrieval"}])).is_none());
}

#[test]
fn a_schema_field_gemini_rejects_is_dropped_rather_than_failing_the_call() {
    // `additionalProperties: false` is on every strict-mode OpenAI tool, and
    // Gemini's parser refuses the whole request over it.
    let schema = json!({
        "type": "object",
        "additionalProperties": false,
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "properties": {"city": {"type": "string"}},
        "required": ["city"],
    });
    let out = schema_for(&schema);
    assert!(out.get("additionalProperties").is_none(), "{out}");
    assert!(out.get("$schema").is_none(), "{out}");
    assert_eq!(out["type"], "object");
    assert_eq!(out["properties"]["city"]["type"], "string");
    assert_eq!(out["required"], json!(["city"]));
}

#[test]
fn a_nested_schema_is_cut_down_too() {
    // A nested object's rejected field fails the request exactly as an outer
    // one does.
    let schema = json!({
        "type": "object",
        "properties": {
            "place": {
                "type": "object",
                "additionalProperties": false,
                "properties": {"city": {"type": "string", "const": "Oslo"}},
            },
            "days": {
                "type": "array",
                "items": {"type": "object", "additionalProperties": false},
            },
            "either": {"anyOf": [{"type": "string", "$ref": "#/x"}, {"type": "number"}]},
        },
    });
    let out = schema_for(&schema);
    assert!(
        out["properties"]["place"]
            .get("additionalProperties")
            .is_none()
    );
    assert!(
        out["properties"]["place"]["properties"]["city"]
            .get("const")
            .is_none()
    );
    assert!(
        out["properties"]["days"]["items"]
            .get("additionalProperties")
            .is_none()
    );
    assert!(
        out["properties"]["either"]["anyOf"][0]
            .get("$ref")
            .is_none()
    );
    assert_eq!(out["properties"]["either"]["anyOf"][1]["type"], "number");
}

#[test]
fn a_tool_choice_becomes_a_function_calling_mode() {
    assert_eq!(
        choice_for(&json!("auto")).expect("translated"),
        json!({"functionCallingConfig": {"mode": "AUTO"}})
    );
    assert_eq!(
        choice_for(&json!("none")).expect("translated"),
        json!({"functionCallingConfig": {"mode": "NONE"}})
    );
    // Different words for the same instruction.
    for required in ["required", "any"] {
        assert_eq!(
            choice_for(&json!(required)).expect("translated"),
            json!({"functionCallingConfig": {"mode": "ANY"}})
        );
    }
    assert!(choice_for(&json!("something_new")).is_none());
}

#[test]
fn naming_one_function_narrows_any_to_it() {
    // Gemini has no single-tool mode of its own.
    let named = choice_for(&json!({"type": "function", "function": {"name": "get_weather"}}))
        .expect("translated");
    assert_eq!(
        named,
        json!({"functionCallingConfig": {"mode": "ANY", "allowedFunctionNames": ["get_weather"]}})
    );
}

#[test]
fn an_assistant_turn_becomes_function_call_parts() {
    let mut conversation = Conversation::default();
    let parts = conversation
        .calls_in(&json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call_0",
                "type": "function",
                "function": {"name": "get_weather", "arguments": "{\"city\":\"Oslo\"}"},
            }],
        }))
        .expect("a call");

    assert_eq!(
        parts,
        vec![json!({"functionCall": {"name": "get_weather", "args": {"city": "Oslo"}}})],
        "arguments are a string for OpenAI and an object here"
    );
}

#[test]
fn a_result_is_matched_to_its_call_by_id_and_sent_by_name() {
    // Gemini's `functionResponse` names the function, and an OpenAI tool
    // result only carries an id.
    let mut conversation = Conversation::default();
    conversation.calls_in(&json!({
        "tool_calls": [{"id": "call_0", "function": {"name": "get_weather", "arguments": "{}"}}],
    }));

    let part = conversation
        .result_in(&json!({"role": "tool", "tool_call_id": "call_0", "content": "sunny"}))
        .expect("a response");
    assert_eq!(
        part,
        json!({"functionResponse": {"name": "get_weather", "response": {"output": "sunny"}}}),
        "a bare string is not a valid `response` and dropping it would answer with nothing"
    );
}

#[test]
fn a_structured_result_is_used_as_it_stands() {
    let mut conversation = Conversation::default();
    conversation.calls_in(&json!({
        "tool_calls": [{"id": "call_0", "function": {"name": "f", "arguments": "{}"}}],
    }));

    let part = conversation
        .result_in(&json!({
            "role": "tool", "tool_call_id": "call_0",
            "content": "{\"temp\": 4, \"sky\": \"clear\"}",
        }))
        .expect("a response");
    assert_eq!(
        part["functionResponse"]["response"],
        json!({"temp": 4, "sky": "clear"}),
        "that is what the tool actually returned"
    );
}

#[test]
fn a_result_for_a_call_that_was_never_announced_is_dropped() {
    // Sending it under a guessed name would have the model read one tool's
    // output as another's.
    let conversation = Conversation::default();
    assert!(
        conversation
            .result_in(&json!({"role": "tool", "tool_call_id": "call_9", "content": "x"}))
            .is_none()
    );
}

#[test]
fn the_deprecated_function_role_carries_its_own_name() {
    let conversation = Conversation::default();
    let part = conversation
        .result_in(&json!({"role": "function", "name": "get_weather", "content": "sunny"}))
        .expect("a response");
    assert_eq!(part["functionResponse"]["name"], "get_weather");
}

#[test]
fn calls_come_back_with_ids_the_client_can_answer_under() {
    // Gemini sends none, and an OpenAI client needs one.
    let candidate = json!({"content": {"role": "model", "parts": [
        {"functionCall": {"name": "get_weather", "args": {"city": "Oslo"}}},
        {"functionCall": {"name": "get_time", "args": {}}},
    ]}});

    let calls = calls_from(&candidate).expect("two calls");
    let calls = calls.as_array().expect("array");
    assert_eq!(calls[0]["id"], "call_0");
    assert_eq!(calls[0]["type"], "function");
    assert_eq!(calls[0]["function"]["name"], "get_weather");
    assert_eq!(calls[0]["function"]["arguments"], r#"{"city":"Oslo"}"#);
    assert_eq!(calls[1]["id"], "call_1");
    assert_eq!(calls[1]["function"]["arguments"], "{}");
}

#[test]
fn a_candidate_that_called_nothing_sends_no_field() {
    // An empty array claims the model considered tools and declined.
    assert!(calls_from(&json!({"content": {"parts": [{"text": "hi"}]}})).is_none());
    assert!(calls_from(&json!({})).is_none());
}

#[test]
fn an_id_this_gateway_handed_out_round_trips() {
    // The ids in a conversation are the ones `calls_from` made up, so a client
    // echoing back what it was given resolves to the right name.
    let candidate = json!({"content": {"parts": [
        {"functionCall": {"name": "get_weather", "args": {}}},
    ]}});
    let calls = calls_from(&candidate).expect("a call");

    let mut conversation = Conversation::default();
    conversation.calls_in(&json!({"tool_calls": calls}));
    let part = conversation
        .result_in(&json!({"role": "tool", "tool_call_id": call_id(0), "content": "sunny"}))
        .expect("a response");
    assert_eq!(part["functionResponse"]["name"], "get_weather");
}

#[test]
fn arguments_a_model_garbled_become_an_empty_object() {
    // Failing a whole conversation over one call is worse than forwarding a
    // call the model can be told went wrong.
    let mut conversation = Conversation::default();
    let parts = conversation
        .calls_in(&json!({
            "tool_calls": [{"id": "a", "function": {"name": "f", "arguments": "{\"city\":"}}],
        }))
        .expect("a call");
    assert_eq!(parts[0]["functionCall"]["args"], json!({}));
}

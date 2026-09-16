//! Unit tests for tool-call translation.

use super::*;

fn openai_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Look up the weather",
            "parameters": {
                "type": "object",
                "properties": {"city": {"type": "string"}},
                "required": ["city"],
            },
        },
    })
}

#[test]
fn a_definition_is_flattened_and_the_schema_renamed() {
    let translated =
        definitions_to_anthropic(&json!([openai_tool()])).expect("should translate one");
    let tools = translated.as_array().expect("an array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "get_weather");
    assert_eq!(tools[0]["description"], "Look up the weather");
    assert_eq!(
        tools[0]["input_schema"]["properties"]["city"]["type"], "string",
        "`parameters` is `input_schema`: {}",
        tools[0]
    );
    assert!(
        tools[0].get("function").is_none(),
        "the wrapper is gone: {}",
        tools[0]
    );
}

#[test]
fn a_tool_with_no_parameters_still_gets_a_schema() {
    // Anthropic requires `input_schema`, and a tool taking nothing still has
    // one: the empty object.
    let translated = definitions_to_anthropic(&json!([{
        "type": "function",
        "function": {"name": "ping"},
    }]))
    .expect("should translate");
    assert_eq!(translated[0]["input_schema"]["type"], "object");
}

#[test]
fn an_empty_tool_list_translates_to_nothing_at_all() {
    // Anthropic reads an empty `tools` array as "use no tools", which is not
    // the same as not mentioning them.
    assert!(definitions_to_anthropic(&json!([])).is_none());
}

#[test]
fn a_tool_kind_anthropic_has_no_shape_for_is_skipped_not_mangled() {
    let translated = definitions_to_anthropic(&json!([
        {"type": "custom", "custom": {"name": "x"}},
        openai_tool(),
    ]))
    .expect("the function survives");
    assert_eq!(translated.as_array().expect("an array").len(), 1);
    assert_eq!(translated[0]["name"], "get_weather");
}

#[test]
fn every_tool_choice_spelling_maps() {
    assert_eq!(
        choice_to_anthropic(&json!("auto")),
        Some(json!({"type": "auto"}))
    );
    assert_eq!(
        choice_to_anthropic(&json!("none")),
        Some(json!({"type": "none"}))
    );
    // Different words for the same instruction.
    assert_eq!(
        choice_to_anthropic(&json!("required")),
        Some(json!({"type": "any"}))
    );
    assert_eq!(
        choice_to_anthropic(&json!({"type": "function", "function": {"name": "get_weather"}})),
        Some(json!({"type": "tool", "name": "get_weather"}))
    );
}

#[test]
fn a_tool_choice_this_build_does_not_know_is_dropped_rather_than_guessed() {
    assert!(choice_to_anthropic(&json!("something_new")).is_none());
    assert!(choice_to_anthropic(&json!(42)).is_none());
}

#[test]
fn an_assistant_tool_call_becomes_a_content_block() {
    let message = json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [{
            "id": "call_1",
            "type": "function",
            "function": {"name": "get_weather", "arguments": "{\"city\":\"Oslo\"}"},
        }],
    });
    let blocks = assistant_to_anthropic(&message).expect("should translate");
    let blocks = blocks.as_array().expect("an array");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["type"], "tool_use");
    assert_eq!(blocks[0]["id"], "call_1");
    assert_eq!(blocks[0]["name"], "get_weather");
    assert_eq!(
        blocks[0]["input"],
        json!({"city": "Oslo"}),
        "the argument string is parsed into an object"
    );
}

#[test]
fn text_beside_a_tool_call_keeps_its_place_in_front() {
    // Anthropic puts the model's own words and its calls in one block list, in
    // the order it produced them.
    let message = json!({
        "role": "assistant",
        "content": "Let me look that up.",
        "tool_calls": [{
            "id": "call_1",
            "function": {"name": "get_weather", "arguments": "{}"},
        }],
    });
    let blocks = assistant_to_anthropic(&message).expect("should translate");
    let blocks = blocks.as_array().expect("an array");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[0]["text"], "Let me look that up.");
    assert_eq!(blocks[1]["type"], "tool_use");
}

#[test]
fn an_assistant_turn_with_no_calls_is_left_for_the_caller_to_keep() {
    // `None` so the caller keeps the content it already had rather than
    // rebuilding an identical thing.
    assert!(assistant_to_anthropic(&json!({"role": "assistant", "content": "hi"})).is_none());
    assert!(
        assistant_to_anthropic(&json!({"role": "assistant", "tool_calls": []})).is_none(),
        "an empty list is not a call"
    );
}

#[test]
fn a_garbled_argument_string_becomes_an_empty_object_rather_than_an_error() {
    // Failing a whole conversation over one call a model garbled is worse than
    // forwarding a call the model can be told went wrong.
    let message = json!({
        "role": "assistant",
        "tool_calls": [{
            "id": "call_1",
            "function": {"name": "f", "arguments": "{\"city\": \"Os"},
        }],
    });
    let blocks = assistant_to_anthropic(&message).expect("should translate");
    assert_eq!(blocks[0]["input"], json!({}));
}

#[test]
fn a_tool_result_becomes_a_result_block() {
    let block = result_to_anthropic(&json!({
        "role": "tool",
        "tool_call_id": "call_1",
        "content": "17 degrees",
    }));
    assert_eq!(
        block,
        json!({
            "type": "tool_result",
            "tool_use_id": "call_1",
            "content": "17 degrees",
        })
    );
}

#[test]
fn a_structured_tool_result_is_serialized_rather_than_dropped() {
    // The model reads it either way.
    let block = result_to_anthropic(&json!({
        "role": "tool",
        "tool_call_id": "call_1",
        "content": {"celsius": 17},
    }));
    assert_eq!(block["content"], r#"{"celsius":17}"#);
}

#[test]
fn an_anthropic_tool_use_block_becomes_an_openai_call() {
    let calls = calls_from_anthropic(&json!([
        {"type": "text", "text": "Looking that up."},
        {"type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": {"city": "Oslo"}},
    ]))
    .expect("should find one");
    let calls = calls.as_array().expect("an array");
    assert_eq!(calls.len(), 1, "text blocks are not calls");
    assert_eq!(calls[0]["id"], "toolu_1");
    assert_eq!(calls[0]["type"], "function");
    assert_eq!(calls[0]["function"]["name"], "get_weather");
    assert_eq!(
        calls[0]["function"]["arguments"], r#"{"city":"Oslo"}"#,
        "the input object becomes an argument string"
    );
}

#[test]
fn several_tool_use_blocks_all_come_back() {
    let calls = calls_from_anthropic(&json!([
        {"type": "tool_use", "id": "a", "name": "one", "input": {}},
        {"type": "tool_use", "id": "b", "name": "two", "input": {}},
    ]))
    .expect("should find two");
    assert_eq!(calls.as_array().expect("an array").len(), 2);
}

#[test]
fn a_response_with_no_calls_reports_none_rather_than_an_empty_list() {
    // An empty array is a claim that the model considered tools and declined,
    // which is not what happened.
    assert!(calls_from_anthropic(&json!([{"type": "text", "text": "hi"}])).is_none());
    assert!(calls_from_anthropic(&json!([])).is_none());
    assert!(calls_from_anthropic(&json!("not a list")).is_none());
}

#[test]
fn a_call_survives_the_whole_round_trip() {
    // Out as a content block, back as an OpenAI call, with the arguments
    // intact through both string/object conversions.
    let outbound = assistant_to_anthropic(&json!({
        "role": "assistant",
        "tool_calls": [{
            "id": "call_1",
            "function": {"name": "f", "arguments": "{\"n\":1}"},
        }],
    }))
    .expect("should translate out");

    let back = calls_from_anthropic(&outbound).expect("should translate back");
    assert_eq!(back[0]["id"], "call_1");
    assert_eq!(back[0]["function"]["name"], "f");
    assert_eq!(back[0]["function"]["arguments"], r#"{"n":1}"#);
}

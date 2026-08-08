//! Unit tests for the Gemini translation.

use serde_json::json;

use super::*;

fn request() -> Value {
    json!({
        "model": "gemini-2.5-flash",
        "messages": [
            {"role": "system", "content": "Be brief."},
            {"role": "user", "content": "Hello"},
        ],
        "temperature": 0.2,
    })
}

#[test]
fn a_bare_name_and_a_prefixed_one_reach_the_same_path() {
    // The API's own examples use both spellings, so one is normalised rather
    // than being a way to write the same request wrong.
    assert_eq!(
        model_path("gemini-2.5-flash").expect("usable"),
        "models/gemini-2.5-flash"
    );
    assert_eq!(
        model_path("models/gemini-2.5-flash").expect("usable"),
        "models/gemini-2.5-flash"
    );
}

#[test]
fn a_model_name_that_would_choose_a_different_endpoint_is_refused() {
    // This is the one place a client's string reaches a URL the gateway signs
    // with its own key. A name carrying a separator picks another method or
    // another resource on the same host.
    for hostile in [
        "../tunedModels/private",
        "gemini-2.5-flash:countTokens",
        "gemini/../../v1beta/models/other",
        "gemini 2.5",
        "",
        "models/",
        "gemini?key=leak",
    ] {
        assert!(
            model_path(hostile).is_err(),
            "`{hostile}` should not reach a URL"
        );
    }
}

#[test]
fn the_system_prompt_becomes_a_system_instruction() {
    let out = to_gemini(&request()).expect("should translate");
    assert_eq!(out["systemInstruction"]["parts"][0]["text"], "Be brief.");
    assert!(
        out["systemInstruction"].get("role").is_none(),
        "an instruction is not a turn, and Gemini rejects a role here"
    );

    let contents = out["contents"].as_array().expect("contents");
    assert_eq!(contents.len(), 1, "the system turn is not also a content");
    assert_eq!(contents[0]["role"], "user");
    assert_eq!(contents[0]["parts"][0]["text"], "Hello");
}

#[test]
fn several_system_prompts_are_joined_rather_than_dropped() {
    let mut body = request();
    body["messages"] = json!([
        {"role": "system", "content": "First."},
        {"role": "developer", "content": "Second."},
        {"role": "user", "content": "Hi"},
    ]);
    let out = to_gemini(&body).expect("should translate");
    assert_eq!(
        out["systemInstruction"]["parts"][0]["text"],
        "First.\n\nSecond."
    );
}

#[test]
fn the_assistant_role_is_called_model() {
    let mut body = request();
    body["messages"] = json!([
        {"role": "user", "content": "Hi"},
        {"role": "assistant", "content": "Hello"},
        {"role": "user", "content": "How are you?"},
    ]);
    let roles: Vec<String> = to_gemini(&body).expect("should translate")["contents"]
        .as_array()
        .expect("contents")
        .iter()
        .map(|turn| turn["role"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(roles, vec!["user", "model", "user"]);
}

#[test]
fn two_turns_of_one_role_are_joined_rather_than_sent_twice() {
    // Gemini rejects them, and a caller that sent two user messages meant
    // both: dropping one would silently change the question.
    let mut body = request();
    body["messages"] = json!([
        {"role": "user", "content": "First half."},
        {"role": "user", "content": "Second half."},
    ]);
    let out = to_gemini(&body).expect("should translate");
    let contents = out["contents"].as_array().expect("contents");
    assert_eq!(contents.len(), 1);
    assert_eq!(
        contents[0]["parts"],
        json!([{"text": "First half."}, {"text": "Second half."}])
    );
}

#[test]
fn sampling_settings_move_into_generation_config() {
    // Carried over by name they would produce a request Gemini accepts and
    // quietly ignores, which is worse than one it rejects.
    let mut body = request();
    body["top_p"] = json!(0.9);
    body["max_tokens"] = json!(256);
    body["stop"] = json!("END");

    let config = &to_gemini(&body).expect("should translate")["generationConfig"];
    assert_eq!(config["temperature"], 0.2);
    assert_eq!(config["topP"], 0.9);
    assert_eq!(config["maxOutputTokens"], 256);
    assert_eq!(
        config["stopSequences"],
        json!(["END"]),
        "a list even for one sequence"
    );
}

#[test]
fn the_newer_max_completion_tokens_spelling_is_honoured() {
    let mut body = request();
    body["max_completion_tokens"] = json!(50);
    let out = to_gemini(&body).expect("should translate");
    assert_eq!(out["generationConfig"]["maxOutputTokens"], 50);
}

#[test]
fn a_request_with_nothing_to_configure_sends_no_config_at_all() {
    let mut body = request();
    body.as_object_mut().expect("object").remove("temperature");
    let out = to_gemini(&body).expect("should translate");
    assert!(
        out.get("generationConfig").is_none(),
        "an empty object on every request is noise: {out}"
    );
}

#[test]
fn the_model_and_the_stream_flag_are_left_out_of_the_body() {
    // The first is in the URL and the second is a different method there.
    // Gemini rejects a request holding fields it does not know.
    let out = to_gemini(&request()).expect("should translate");
    assert!(out.get("model").is_none(), "{out}");
    assert!(out.get("stream").is_none(), "{out}");
    assert!(out.get("messages").is_none(), "{out}");
}

#[test]
fn a_streaming_request_is_refused_rather_than_answered_in_one_piece() {
    let mut body = request();
    body["stream"] = json!(true);
    let err = to_gemini(&body).expect_err("should not translate");
    assert!(err.to_string().contains("streaming"), "{err}");
    assert!(err.to_string().contains("gemini"), "{err}");
}

#[test]
fn a_request_asking_for_tools_is_refused_rather_than_stripped() {
    // Dropping them would look like a model that chose not to call one, which
    // is far more expensive to debug than a refusal.
    let mut with_tools = request();
    with_tools["tools"] = json!([{"type": "function", "function": {"name": "f"}}]);
    assert!(to_gemini(&with_tools).is_err());

    let mut with_choice = request();
    with_choice["tool_choice"] = json!("auto");
    assert!(to_gemini(&with_choice).is_err());

    let mut with_result = request();
    with_result["messages"] = json!([
        {"role": "user", "content": "weather?"},
        {"role": "tool", "tool_call_id": "call_1", "content": "sunny"},
    ]);
    assert!(to_gemini(&with_result).is_err());
}

#[test]
fn a_null_tools_field_is_not_a_request_for_tools() {
    // Clients that always send the key and leave it null are common, and
    // refusing them would refuse ordinary traffic.
    let mut body = request();
    body["tools"] = Value::Null;
    assert!(to_gemini(&body).is_ok());
}

#[test]
fn the_text_parts_of_a_multimodal_message_carry_across() {
    let mut body = request();
    body["messages"] = json!([{"role": "user", "content": [
        {"type": "text", "text": "what is this"},
        {"type": "image_url", "image_url": {"url": "https://example.com/a.png"}},
    ]}]);
    let out = to_gemini(&body).expect("should translate");
    assert_eq!(
        out["contents"][0]["parts"],
        json!([{"text": "what is this"}]),
        "an image URL has no honest translation: Gemini takes bytes or a file URI"
    );
}

#[test]
fn a_turn_with_nothing_in_it_is_dropped_rather_than_sent_empty() {
    // Gemini rejects an empty `parts`.
    let mut body = request();
    body["messages"] = json!([
        {"role": "user", "content": ""},
        {"role": "assistant", "content": null},
        {"role": "user", "content": "Real question"},
    ]);
    let out = to_gemini(&body).expect("should translate");
    let contents = out["contents"].as_array().expect("contents");
    assert_eq!(contents.len(), 1);
    assert_eq!(contents[0]["parts"][0]["text"], "Real question");
}

#[test]
fn a_body_without_messages_is_refused() {
    let err = to_gemini(&json!({"model": "x"})).expect_err("should not translate");
    assert!(matches!(err, TranslateError::Messages));
}

fn response() -> Value {
    json!({
        "responseId": "resp-1",
        "modelVersion": "gemini-2.5-flash",
        "candidates": [{
            "content": {"role": "model", "parts": [{"text": "Hello"}, {"text": " there"}]},
            "finishReason": "STOP",
            "index": 0,
        }],
        "usageMetadata": {
            "promptTokenCount": 12,
            "candidatesTokenCount": 5,
            "totalTokenCount": 17,
        },
    })
}

#[test]
fn a_response_translates_into_openai_shape() {
    let out = from_gemini(&response(), 1_700_000_000);
    assert_eq!(out["object"], "chat.completion");
    assert_eq!(out["id"], "resp-1");
    assert_eq!(out["model"], "gemini-2.5-flash");
    assert_eq!(out["created"], 1_700_000_000u64);
    assert_eq!(
        out["choices"][0]["message"]["content"], "Hello there",
        "several parts are one message to an OpenAI client"
    );
    assert_eq!(out["choices"][0]["message"]["role"], "assistant");
    assert_eq!(out["choices"][0]["finish_reason"], "stop");
}

#[test]
fn usage_is_renamed_and_totalled() {
    let out = from_gemini(&response(), 0);
    assert_eq!(out["usage"]["prompt_tokens"], 12);
    assert_eq!(out["usage"]["completion_tokens"], 5);
    assert_eq!(out["usage"]["total_tokens"], 17);
}

#[test]
fn a_refused_answer_still_reports_the_prompt_it_read() {
    // `candidatesTokenCount` is absent when nothing was generated. The prompt
    // was still read and still billed.
    let refused = json!({
        "candidates": [{"finishReason": "SAFETY"}],
        "usageMetadata": {"promptTokenCount": 30, "totalTokenCount": 30},
    });
    assert_eq!(
        usage(&refused),
        Some(Usage {
            prompt: 30,
            completion: 0
        })
    );
    let out = from_gemini(&refused, 0);
    assert_eq!(out["choices"][0]["finish_reason"], "content_filter");
    assert_eq!(
        out["choices"][0]["message"]["content"],
        Value::Null,
        "null rather than empty, matching what OpenAI itself sends"
    );
}

#[test]
fn finish_reasons_are_mapped_to_openai_vocabulary() {
    assert_eq!(finish_reason(Some("STOP")), json!("stop"));
    assert_eq!(finish_reason(Some("MAX_TOKENS")), json!("length"));
    // Reporting a cut-off answer as a clean stop would tell a client the model
    // finished its thought.
    for refused in [
        "SAFETY",
        "BLOCKLIST",
        "PROHIBITED_CONTENT",
        "SPII",
        "RECITATION",
    ] {
        assert_eq!(finish_reason(Some(refused)), json!("content_filter"));
    }
    assert_eq!(
        finish_reason(None),
        Value::Null,
        "null is what OpenAI sends mid-stream"
    );
    assert_eq!(finish_reason(Some("SOMETHING_NEW")), json!("stop"));
}

#[test]
fn a_response_with_no_candidates_does_not_panic() {
    let out = from_gemini(&json!({"usageMetadata": {"promptTokenCount": 4}}), 0);
    assert_eq!(out["choices"][0]["message"]["content"], Value::Null);
    assert_eq!(out["choices"][0]["finish_reason"], Value::Null);
    assert_eq!(out["usage"]["prompt_tokens"], 4);
}

#[test]
fn usage_is_absent_rather_than_zero_when_the_provider_sent_none() {
    assert_eq!(usage(&json!({})), None);
}

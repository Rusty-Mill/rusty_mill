//! Unit tests for cache breakpoints.

use super::*;

fn policy(system: bool, messages: bool) -> PromptCaching {
    PromptCaching {
        cache_system: system,
        cache_messages: messages,
        cache_tools: false,
        min_tokens: None,
        cache_message_offset: None,
    }
}

fn caching(policy: PromptCaching) -> Caching {
    Caching::new(Some(&policy)).expect("should compile to something")
}

/// An Anthropic Messages request as `to_anthropic` produces one.
fn request() -> Value {
    json!({
        "model": "claude-sonnet-4",
        "system": "You are terse.",
        "messages": [
            {"role": "user", "content": "one"},
            {"role": "assistant", "content": "two"},
            {"role": "user", "content": "three"},
        ],
        "max_tokens": 1024,
    })
}

#[test]
fn a_policy_that_would_mark_nothing_compiles_to_nothing() {
    assert!(Caching::new(None).is_none());
    assert!(Caching::new(Some(&policy(false, false))).is_none());
    let mut only_tools = policy(false, false);
    only_tools.cache_tools = true;
    assert!(
        Caching::new(Some(&only_tools)).is_some(),
        "`cacheTools` alone is now something to do"
    );
}

#[test]
fn a_tool_definition_can_carry_a_breakpoint() {
    // Tools sit ahead of everything else Anthropic caches, so this is the
    // cheapest breakpoint to set and the likeliest to hit.
    let mut configured = policy(false, false);
    configured.cache_tools = true;
    let caching = caching(configured);

    let mut body = request();
    body["tools"] = json!([
        {"name": "one", "input_schema": {}},
        {"name": "two", "input_schema": {}},
    ]);
    caching.apply(&mut body);

    let tools = body["tools"].as_array().expect("an array");
    assert!(tools[0].get("cache_control").is_none());
    assert_eq!(tools[1]["cache_control"], json!({"type": "ephemeral"}));
}

#[test]
fn a_request_with_no_tools_is_left_alone() {
    let mut configured = policy(false, false);
    configured.cache_tools = true;
    let caching = caching(configured);

    let mut body = request();
    caching.apply(&mut body);
    assert!(body.get("tools").is_none(), "{body}");
}

#[test]
fn a_string_system_prompt_is_promoted_to_a_block_it_can_be_marked_on() {
    let caching = caching(policy(true, false));
    let mut body = request();
    caching.apply(&mut body);

    assert_eq!(
        body["system"],
        json!([{
            "type": "text",
            "text": "You are terse.",
            "cache_control": {"type": "ephemeral"},
        }])
    );
}

#[test]
fn a_block_system_prompt_is_marked_on_its_last_block() {
    // The breakpoint covers everything up to where it sits, so the last block
    // is the one that caches the whole list.
    let caching = caching(policy(true, false));
    let mut body = request();
    body["system"] = json!([
        {"type": "text", "text": "one"},
        {"type": "text", "text": "two"},
    ]);
    caching.apply(&mut body);

    let blocks = body["system"].as_array().expect("an array");
    assert!(blocks[0].get("cache_control").is_none());
    assert_eq!(blocks[1]["cache_control"], json!({"type": "ephemeral"}));
}

#[test]
fn the_last_message_is_marked_by_default() {
    let caching = caching(policy(false, true));
    let mut body = request();
    caching.apply(&mut body);

    let messages = body["messages"].as_array().expect("an array");
    assert!(messages[0]["content"].is_string(), "untouched");
    assert_eq!(
        messages[2]["content"],
        json!([{
            "type": "text",
            "text": "three",
            "cache_control": {"type": "ephemeral"},
        }])
    );
}

#[test]
fn an_offset_counts_back_from_the_end() {
    // A conversation grows a turn at the end, so the breakpoint wants to sit
    // behind the part that changes.
    let mut configured = policy(false, true);
    configured.cache_message_offset = Some(1);
    let caching = caching(configured);

    let mut body = request();
    caching.apply(&mut body);

    let messages = body["messages"].as_array().expect("an array");
    assert!(messages[1]["content"].is_array(), "the middle turn");
    assert!(messages[2]["content"].is_string(), "the newest turn is not");
}

#[test]
fn an_offset_past_the_start_marks_the_first_message() {
    // "As far back as asked" -- refusing to mark anything would be a silent
    // no-op for a config that clearly wanted a breakpoint.
    let mut configured = policy(false, true);
    configured.cache_message_offset = Some(99);
    let caching = caching(configured);

    let mut body = request();
    caching.apply(&mut body);

    let messages = body["messages"].as_array().expect("an array");
    assert!(messages[0]["content"].is_array());
}

#[test]
fn a_prompt_under_min_tokens_is_left_unmarked() {
    let mut configured = policy(true, true);
    configured.min_tokens = Some(100_000);
    let caching = caching(configured);

    let mut body = request();
    caching.apply(&mut body);

    assert!(body["system"].is_string(), "{body}");
    assert!(body["messages"][2]["content"].is_string(), "{body}");
}

#[test]
fn a_prompt_over_min_tokens_is_marked() {
    let mut configured = policy(true, false);
    configured.min_tokens = Some(10);
    let caching = caching(configured);

    let mut body = request();
    body["system"] = json!("x".repeat(1000));
    caching.apply(&mut body);

    assert!(body["system"].is_array(), "{}", body["system"]);
}

#[test]
fn an_empty_message_list_is_not_indexed_into() {
    let caching = caching(policy(false, true));
    let mut body = json!({"messages": [], "model": "claude-sonnet-4"});
    caching.apply(&mut body);
    assert_eq!(body["messages"], json!([]));
}

#[test]
fn a_request_with_no_system_prompt_is_left_alone() {
    let caching = caching(policy(true, false));
    let mut body = json!({"messages": [{"role": "user", "content": "hi"}]});
    caching.apply(&mut body);
    assert!(body.get("system").is_none(), "{body}");
}

#[test]
fn both_breakpoints_can_be_set_at_once() {
    let caching = caching(policy(true, true));
    let mut body = request();
    caching.apply(&mut body);

    assert!(body["system"].is_array());
    assert!(body["messages"][2]["content"].is_array());
}

//! Unit tests for request shaping.

use agentgateway_config::Prompts;
use serde_json::json;

use super::*;

fn policy(build: impl FnOnce(&mut AiPolicy)) -> AiPolicy {
    let mut policy = AiPolicy::default();
    build(&mut policy);
    policy
}

fn shape(policy: AiPolicy) -> Shape {
    Shape::new(Some(&policy)).expect("should compile to something")
}

fn prompt(role: &str, content: &str) -> PromptMessage {
    PromptMessage {
        role: role.to_string(),
        content: content.to_string(),
    }
}

fn request() -> Value {
    json!({
        "model": "fast",
        "messages": [{"role": "user", "content": "Hello"}],
    })
}

#[test]
fn an_empty_policy_compiles_to_nothing() {
    // So the request path can skip the work rather than walk four empty
    // collections per call.
    assert!(Shape::new(None).is_none());
    assert!(Shape::new(Some(&AiPolicy::default())).is_none());
}

#[test]
fn a_policy_of_only_unimplemented_keys_compiles_to_nothing() {
    // `promptGuard` lands in `rest` and is reported by the lint; it must not
    // make the request path do work for a policy it cannot act on.
    let policy = policy(|p| {
        p.rest.insert("promptGuard".into(), json!({"request": []}));
    });
    assert!(Shape::new(Some(&policy)).is_none());
}

#[test]
fn an_alias_resolves_to_the_model_it_stands_for() {
    let shape = shape(policy(|p| {
        p.model_aliases.insert("fast".into(), "gpt-4o-mini".into());
    }));
    let mut body = request();
    shape.apply(&mut body);
    assert_eq!(body["model"], "gpt-4o-mini");
}

#[test]
fn a_name_that_is_not_an_alias_passes_through() {
    // So a route can alias `fast` without enumerating every model a caller
    // might otherwise ask for.
    let shape = shape(policy(|p| {
        p.model_aliases.insert("fast".into(), "gpt-4o-mini".into());
    }));
    let mut body = json!({"model": "gpt-4o", "messages": []});
    shape.apply(&mut body);
    assert_eq!(body["model"], "gpt-4o");
}

#[test]
fn an_alias_is_not_followed_twice() {
    // `a -> b -> c` resolves to `b`. A chain would let a config loop, and a
    // gateway that hangs on its own configuration is worse than one that
    // resolves one step.
    let shape = shape(policy(|p| {
        p.model_aliases.insert("a".into(), "b".into());
        p.model_aliases.insert("b".into(), "c".into());
    }));
    let mut body = json!({"model": "a", "messages": []});
    shape.apply(&mut body);
    assert_eq!(body["model"], "b");
}

#[test]
fn prepended_messages_come_before_the_callers() {
    let shape = shape(policy(|p| {
        p.prompts = Some(Prompts {
            prepend: vec![prompt("system", "Be brief.")],
            append: Vec::new(),
        });
    }));
    let mut body = request();
    shape.apply(&mut body);

    let messages = body["messages"].as_array().expect("an array");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "Be brief.");
    assert_eq!(messages[1]["content"], "Hello");
}

#[test]
fn appended_messages_come_after_the_callers() {
    let shape = shape(policy(|p| {
        p.prompts = Some(Prompts {
            prepend: Vec::new(),
            append: vec![prompt("system", "Answer in English.")],
        });
    }));
    let mut body = request();
    shape.apply(&mut body);

    let messages = body["messages"].as_array().expect("an array");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["content"], "Hello");
    assert_eq!(messages[1]["content"], "Answer in English.");
}

#[test]
fn several_prepended_messages_keep_their_configured_order() {
    let shape = shape(policy(|p| {
        p.prompts = Some(Prompts {
            prepend: vec![prompt("system", "one"), prompt("system", "two")],
            append: Vec::new(),
        });
    }));
    let mut body = request();
    shape.apply(&mut body);

    let messages = body["messages"].as_array().expect("an array");
    assert_eq!(messages[0]["content"], "one");
    assert_eq!(messages[1]["content"], "two");
    assert_eq!(messages[2]["content"], "Hello");
}

#[test]
fn a_body_without_messages_is_left_alone_rather_than_given_some() {
    // Inventing the array would turn a client bug into a request that runs --
    // with only the operator's prompt in it.
    let shape = shape(policy(|p| {
        p.prompts = Some(Prompts {
            prepend: vec![prompt("system", "Be brief.")],
            append: Vec::new(),
        });
    }));
    let mut body = json!({"model": "gpt-4o"});
    shape.apply(&mut body);
    assert!(body.get("messages").is_none(), "{body}");
}

#[test]
fn a_default_fills_a_field_the_caller_left_out() {
    let shape = shape(policy(|p| {
        p.defaults.insert("temperature".into(), json!(0.2));
    }));
    let mut body = request();
    shape.apply(&mut body);
    assert_eq!(body["temperature"], 0.2);
}

#[test]
fn a_default_does_not_replace_what_the_caller_sent() {
    let shape = shape(policy(|p| {
        p.defaults.insert("temperature".into(), json!(0.2));
    }));
    let mut body = request();
    body["temperature"] = json!(0.9);
    shape.apply(&mut body);
    assert_eq!(body["temperature"], 0.9, "a default is not an override");
}

#[test]
fn a_default_of_null_still_counts_as_the_caller_having_answered() {
    // `temperature: null` is a value the caller sent, not an absence. Treating
    // it as missing would quietly disagree with the JSON they wrote.
    let shape = shape(policy(|p| {
        p.defaults.insert("temperature".into(), json!(0.2));
    }));
    let mut body = request();
    body["temperature"] = Value::Null;
    shape.apply(&mut body);
    assert_eq!(body["temperature"], Value::Null);
}

#[test]
fn an_override_replaces_what_the_caller_sent() {
    let shape = shape(policy(|p| {
        p.overrides.insert("temperature".into(), json!(0.0));
    }));
    let mut body = request();
    body["temperature"] = json!(0.9);
    shape.apply(&mut body);
    assert_eq!(body["temperature"], 0.0);
}

#[test]
fn an_override_beats_a_default_for_the_same_field() {
    // The ladder: defaults fill what is missing, overrides replace what is
    // there, so an override applied after a default wins.
    let shape = shape(policy(|p| {
        p.defaults.insert("max_tokens".into(), json!(256));
        p.overrides.insert("max_tokens".into(), json!(1024));
    }));
    let mut body = request();
    shape.apply(&mut body);
    assert_eq!(body["max_tokens"], 1024);
}

#[test]
fn shaping_runs_in_order_and_the_pieces_compose() {
    let shape = shape(policy(|p| {
        p.model_aliases.insert("fast".into(), "gpt-4o-mini".into());
        p.prompts = Some(Prompts {
            prepend: vec![prompt("system", "Be brief.")],
            append: vec![prompt("user", "Thanks.")],
        });
        p.defaults.insert("temperature".into(), json!(0.2));
        p.overrides.insert("max_tokens".into(), json!(512));
    }));
    let mut body = request();
    shape.apply(&mut body);

    assert_eq!(body["model"], "gpt-4o-mini");
    assert_eq!(body["temperature"], 0.2);
    assert_eq!(body["max_tokens"], 512);
    let messages = body["messages"].as_array().expect("an array");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["content"], "Be brief.");
    assert_eq!(messages[2]["content"], "Thanks.");
}

#[test]
fn a_body_that_is_not_an_object_is_left_alone() {
    let shape = shape(policy(|p| {
        p.overrides.insert("temperature".into(), json!(0.0));
    }));
    let mut body = json!([1, 2, 3]);
    shape.apply(&mut body);
    assert_eq!(body, json!([1, 2, 3]));
}

#[test]
fn an_override_can_set_a_field_this_crate_has_never_heard_of() {
    // The same reason the body is forwarded rather than modelled: a provider
    // feature newer than this build must still be reachable.
    let shape = shape(policy(|p| {
        p.overrides.insert("reasoning_effort".into(), json!("high"));
    }));
    let mut body = request();
    shape.apply(&mut body);
    assert_eq!(body["reasoning_effort"], "high");
}

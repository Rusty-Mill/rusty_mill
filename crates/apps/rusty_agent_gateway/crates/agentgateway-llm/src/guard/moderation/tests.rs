//! Unit tests for the moderation rule.

use agentgateway_config::ModerationPolicies;
use serde_json::json;

use super::*;

fn rule(auth: Option<BackendAuth>) -> agentgateway_config::Moderation {
    agentgateway_config::Moderation {
        model: None,
        policies: auth.map(|auth| ModerationPolicies {
            backend_auth: Some(auth),
            rest: Default::default(),
        }),
    }
}

fn rejection() -> Rejection {
    Rejection {
        status: 403,
        headers: None,
        body: None,
    }
}

fn compile(
    config: &agentgateway_config::Moderation,
    borrowable: Option<Borrowable<'_>>,
    provider: &'static str,
) -> Result<Moderation, CredentialError> {
    Moderation::new(config, rejection(), borrowable, provider, "route[0]")
}

#[test]
fn a_rule_with_its_own_key_calls_openai_itself() {
    // Its key was issued for OpenAI, so OpenAI is where it goes -- whatever
    // the route in front of it points at.
    let compiled = compile(
        &rule(Some(BackendAuth::Key("sk-rule".into()))),
        Some(Borrowable {
            endpoint: "http://vllm:8000/v1/moderations",
            key: Some("not-an-openai-key"),
        }),
        "openai",
    )
    .expect("a rule with a key should compile");

    assert_eq!(compiled.endpoint, "https://api.openai.com/v1/moderations");
    assert_eq!(compiled.key, "sk-rule");
}

#[test]
fn a_borrowed_key_goes_no_further_than_the_route_it_came_from() {
    // The narrow divergence from upstream, and the reason for it: a key
    // configured for one host should not be sent to another.
    let compiled = compile(
        &rule(None),
        Some(Borrowable {
            endpoint: "http://127.0.0.1:9/v1/moderations",
            key: Some("sk-route"),
        }),
        "openai",
    )
    .expect("an openAI route with a key should lend it");

    assert_eq!(compiled.endpoint, "http://127.0.0.1:9/v1/moderations");
    assert_eq!(compiled.key, "sk-route");
}

#[test]
fn a_route_on_another_provider_does_not_lend_its_key() {
    // The failure this refusal exists to prevent: an Anthropic key sent to
    // OpenAI is a secret handed to a third party.
    let err = compile(&rule(None), None, "anthropic")
        .expect_err("a non-OpenAI route must not supply an OpenAI credential");

    let message = err.to_string();
    assert!(message.contains("anthropic"), "{message}");
    assert!(message.contains("third party"), "{message}");
}

#[test]
fn an_openai_route_with_no_key_at_all_says_so_differently() {
    // Nothing to borrow rather than nothing borrowable, and the fix differs:
    // give either the route or the rule a key.
    let err = compile(
        &rule(None),
        Some(Borrowable {
            endpoint: "https://api.openai.com/v1/moderations",
            key: None,
        }),
        "openai",
    )
    .expect_err("there is no credential anywhere");

    let message = err.to_string();
    assert!(message.contains("borrow"), "{message}");
    assert!(!message.contains("third party"), "{message}");
}

#[test]
fn passthrough_is_refused_rather_than_ignored() {
    // A client's bearer token is not an OpenAI API key, and sending one as the
    // other would forward a user's credential to OpenAI.
    let err = compile(
        &rule(Some(BackendAuth::Passthrough(true))),
        Some(Borrowable {
            endpoint: "https://api.openai.com/v1/moderations",
            key: Some("sk-route"),
        }),
        "openai",
    )
    .expect_err("passthrough has no meaning here");

    assert!(err.to_string().contains("passthrough"), "{err}");
}

#[test]
fn the_default_model_is_upstreams() {
    let compiled = compile(&rule(Some(BackendAuth::Key("sk".into()))), None, "openai")
        .expect("should compile");
    assert_eq!(compiled.model, "omni-moderation-latest");

    let named = agentgateway_config::Moderation {
        model: Some("text-moderation-stable".into()),
        policies: Some(ModerationPolicies {
            backend_auth: Some(BackendAuth::Key("sk".into())),
            rest: Default::default(),
        }),
    };
    assert_eq!(
        compile(&named, None, "openai")
            .expect("should compile")
            .model,
        "text-moderation-stable"
    );
}

#[test]
fn any_flagged_result_is_a_refusal() {
    assert_eq!(
        read_flagged(&json!({"results": [{"flagged": false}, {"flagged": true}]})),
        Some(true)
    );
    assert_eq!(
        read_flagged(&json!({"results": [{"flagged": false}, {"flagged": false}]})),
        Some(false)
    );
}

#[test]
fn a_body_with_no_results_is_not_a_verdict() {
    // Not a pass: nothing classified anything, so this is the unreachable case
    // rather than an answer of "fine".
    assert_eq!(read_flagged(&json!({})), None);
    assert_eq!(read_flagged(&json!({"results": []})), None);
    assert_eq!(read_flagged(&json!({"results": "no"})), None);
}

#[test]
fn a_result_without_a_flag_is_read_as_unflagged() {
    // The field is required by the API. Absent, the safe reading is that this
    // particular result did not fire -- a sibling that did still refuses.
    assert_eq!(read_flagged(&json!({"results": [{}]})), Some(false));
    assert_eq!(
        read_flagged(&json!({"results": [{}, {"flagged": true}]})),
        Some(true)
    );
}

#[test]
fn an_unreachable_classifier_refuses_with_503() {
    // 503, not the rule's own rejection: nothing decided this prompt was
    // unacceptable.
    let refusal = unreachable();
    assert_eq!(refusal.status, 503);
    assert!(refusal.body.is_none());
}

#[test]
fn every_message_text_is_classified_in_order() {
    let inputs = inputs(&json!({"messages": [
        {"role": "system", "content": "Be brief."},
        {"role": "user", "content": "Hello"},
    ]}));
    assert_eq!(inputs, vec!["Be brief.".to_string(), "Hello".to_string()]);
}

#[test]
fn the_text_parts_of_a_multimodal_message_are_classified_too() {
    // More than the `regex` path reads, and deliberately: this only reads, so
    // there is no structure to rewrite, and skipping the list form would let a
    // prompt evade the rule by spelling itself the other way.
    let inputs = inputs(&json!({"messages": [
        {"role": "user", "content": [
            {"type": "text", "text": "what is in this"},
            {"type": "image_url", "image_url": {"url": "https://example.com/a.png"}},
            {"type": "text", "text": "picture"},
        ]},
    ]}));
    assert_eq!(
        inputs,
        vec!["what is in this".to_string(), "picture".to_string()]
    );
}

#[test]
fn a_prompt_with_no_text_is_not_classified_at_all() {
    assert!(inputs(&json!({})).is_empty());
    assert!(inputs(&json!({"messages": []})).is_empty());
    assert!(
        inputs(&json!({"messages": [
            {"role": "user", "content": [{"type": "image_url", "image_url": {"url": "u"}}]},
        ]}))
        .is_empty()
    );
}

#[tokio::test]
async fn nothing_to_classify_is_allowed_without_a_call() {
    // Port 9 is the discard port: reaching it would hang or refuse, and either
    // way this would fail closed. Allowing without calling is the point.
    let compiled = compile(
        &rule(Some(BackendAuth::Key("sk".into()))),
        Some(Borrowable {
            endpoint: "http://127.0.0.1:9/v1/moderations",
            key: Some("sk"),
        }),
        "openai",
    )
    .expect("should compile");

    assert!(matches!(
        compiled.check(Vec::new()).await,
        Decision::Allowed
    ));
}

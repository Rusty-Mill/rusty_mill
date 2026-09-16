//! Unit tests for the guard webhook contract.

use super::*;

fn context() -> Context {
    Context::new(
        [
            ("x-tenant".to_string(), "acme".to_string()),
            ("authorization".to_string(), "Bearer t".to_string()),
        ]
        .into_iter()
        .collect(),
        Some(json!({"sub": "user-123"})),
    )
}

#[test]
fn a_body_with_no_action_is_not_a_pass() {
    // Upstream's schema requires one. A body without it has not answered the
    // question that was asked.
    match read_action(&json!({}), "messages") {
        Verdict::Reject(rejection) => assert_eq!(rejection.status, 503),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn an_action_with_no_body_is_a_pass() {
    assert!(matches!(
        read_action(&json!({"action": {"reason": "looks fine"}}), "messages"),
        Verdict::Pass
    ));
    assert!(matches!(
        read_action(&json!({"action": {}}), "messages"),
        Verdict::Pass
    ));
}

#[test]
fn a_string_body_is_a_refusal_and_an_object_body_is_a_mask() {
    // The two are told apart by the *type* of `body`, because upstream's
    // action enum is untagged.
    match read_action(
        &json!({"action": {"body": "not allowed", "status_code": 451}}),
        "messages",
    ) {
        Verdict::Reject(rejection) => {
            assert_eq!(rejection.status, 451);
            assert_eq!(rejection.body.as_deref(), Some("not allowed"));
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    match read_action(
        &json!({"action": {"body": {"messages": [{"role": "user", "content": "safe"}]}}}),
        "messages",
    ) {
        Verdict::Mask(texts) => assert_eq!(texts, vec!["safe".to_string()]),
        other => panic!("expected a mask, got {other:?}"),
    }
}

#[test]
fn a_refusal_without_a_status_is_a_400() {
    match read_action(&json!({"action": {"body": "no"}}), "messages") {
        Verdict::Reject(rejection) => assert_eq!(rejection.status, 400),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_response_mask_is_read_out_of_choices_rather_than_messages() {
    // Upstream uses one action shape for both phases, and the key differs.
    match read_action(
        &json!({"action": {"body": {"choices": [
            {"message": {"role": "assistant", "content": "redacted"}}
        ]}}}),
        "choices",
    ) {
        Verdict::Mask(texts) => assert_eq!(texts, vec!["redacted".to_string()]),
        other => panic!("expected a mask, got {other:?}"),
    }
}

#[test]
fn several_masked_messages_come_back_in_order() {
    match read_action(
        &json!({"action": {"body": {"messages": [
            {"role": "system", "content": "one"},
            {"role": "user", "content": "two"},
        ]}}}),
        "messages",
    ) {
        Verdict::Mask(texts) => assert_eq!(texts, vec!["one".to_string(), "two".to_string()]),
        other => panic!("expected a mask, got {other:?}"),
    }
}

#[test]
fn a_mask_this_build_cannot_read_refuses_rather_than_passing() {
    // It asked for a rewrite and did not say to what. Treating that as a pass
    // would serve the original text the webhook had just objected to.
    match read_action(&json!({"action": {"body": {"messages": []}}}), "messages") {
        Verdict::Reject(rejection) => assert_eq!(rejection.status, 503),
        other => panic!("expected a refusal, got {other:?}"),
    }
    match read_action(&json!({"action": {"body": {"wrong": 1}}}), "messages") {
        Verdict::Reject(_) => {}
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn failing_closed_is_the_default() {
    let webhook = Webhook::new(&GuardWebhook {
        target: agentgateway_config::WebhookTarget {
            host: "guard:9000".into(),
        },
        ..Default::default()
    });
    assert!(!webhook.fail_open);
    match webhook.unreachable() {
        // 503, not 400: nothing decided this content was unacceptable, and
        // saying it was would send someone to inspect a fine prompt.
        Verdict::Reject(rejection) => assert_eq!(rejection.status, 503),
        other => panic!("an unreachable webhook must not pass, got {other:?}"),
    }
}

#[test]
fn failing_open_has_to_be_asked_for() {
    let webhook = Webhook::new(&GuardWebhook {
        target: agentgateway_config::WebhookTarget {
            host: "guard:9000".into(),
        },
        failure_mode: FailureMode::FailOpen,
        ..Default::default()
    });
    assert!(matches!(webhook.unreachable(), Verdict::Pass));
}

#[test]
fn a_bare_host_port_gets_a_scheme_and_a_spelled_one_is_left_alone() {
    let bare = Webhook::new(&GuardWebhook {
        target: agentgateway_config::WebhookTarget {
            host: "guard:9000".into(),
        },
        ..Default::default()
    });
    assert_eq!(bare.base, "http://guard:9000");

    let spelled = Webhook::new(&GuardWebhook {
        target: agentgateway_config::WebhookTarget {
            host: "https://guard.example.com/".into(),
        },
        ..Default::default()
    });
    assert_eq!(spelled.base, "https://guard.example.com");
}

#[test]
fn a_header_expression_reads_the_callers_request() {
    let context = context();
    assert_eq!(
        context.eval(r#"request.headers["x-tenant"]"#).as_deref(),
        Some("acme")
    );
    assert_eq!(context.eval("jwt.sub").as_deref(), Some("user-123"));
}

#[test]
fn an_expression_that_resolves_to_nothing_produces_no_header() {
    // An empty `x-tenant` is a claim that there is no tenant, which is not
    // what happened.
    let context = context();
    assert!(context.eval(r#"request.headers["absent"]"#).is_none());
    assert!(context.eval("jwt.missing").is_none());
    assert!(context.eval("this is not cel").is_none());
}

#[test]
fn llm_request_is_only_readable_once_it_exists() {
    let context = context();
    assert!(context.eval("llmRequest.model").is_none());

    let with_body = context.with_llm_request(json!({"model": "gpt-4o"}));
    assert_eq!(
        with_body.eval("llmRequest.model").as_deref(),
        Some("gpt-4o")
    );
}

#[test]
fn a_literal_expression_is_a_literal_header() {
    // Upstream's own example uses one for `:path`, so a quoted string has to
    // survive being treated as CEL.
    let context = context();
    assert_eq!(
        context.eval(r#""/api/guardrails/request""#).as_deref(),
        Some("/api/guardrails/request")
    );
}

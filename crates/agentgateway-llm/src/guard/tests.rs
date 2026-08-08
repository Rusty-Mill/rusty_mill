//! Unit tests for `promptGuard.regex`.

use agentgateway_config::{GuardPattern, GuardRule, RegexGuard};
use serde_json::json;

use super::*;

fn rule(action: GuardAction, patterns: Vec<GuardPattern>) -> GuardRule {
    GuardRule {
        regex: Some(RegexGuard {
            action,
            rules: patterns,
        }),
        ..Default::default()
    }
}

fn pattern(source: &str) -> GuardPattern {
    GuardPattern::Pattern(source.to_string())
}

fn guard(request: Vec<GuardRule>, response: Vec<GuardRule>) -> Guard {
    Guard::new(Some(&PromptGuard { request, response }), "t")
        .expect("should compile")
        .expect("should be present")
}

fn ask(text: &str) -> Value {
    json!({"model": "gpt-4o", "messages": [{"role": "user", "content": text}]})
}

fn content(body: &Value) -> &str {
    body["messages"][0]["content"].as_str().expect("a string")
}

#[test]
fn a_policy_with_no_regex_rule_compiles_to_nothing() {
    // A `webhook` rule lands in the catch-all and is reported by the lint; it
    // must not make the request path scan for patterns it does not have.
    assert!(Guard::new(None, "t").expect("ok").is_none());
    let webhook = GuardRule {
        rest: [("webhook".to_string(), json!({"target": {}}))]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    assert!(
        Guard::new(
            Some(&PromptGuard {
                request: vec![webhook],
                response: Vec::new(),
            }),
            "t"
        )
        .expect("ok")
        .is_none()
    );
}

#[test]
fn a_matching_reject_rule_refuses_with_its_own_answer() {
    let mut refusing = rule(GuardAction::Reject, vec![pattern(r"password[=:]\s*\S+")]);
    refusing.rejection = Some(agentgateway_config::Rejection {
        status: 422,
        headers: None,
        body: Some("no credentials please".into()),
    });
    let guard = guard(vec![refusing], Vec::new());

    let mut body = ask("my password= hunter2");
    match guard.check_request(&mut body) {
        Decision::Rejected(rejection) => {
            assert_eq!(rejection.status, 422);
            assert_eq!(rejection.body.as_deref(), Some("no credentials please"));
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_refusal_without_a_configured_status_is_a_400() {
    // A content rule decides the request is unacceptable for this route, which
    // is what a bad request means. A 403 would send someone to check
    // credentials that are fine.
    let guard = guard(
        vec![rule(GuardAction::Reject, vec![pattern("secret")])],
        Vec::new(),
    );
    match guard.check_request(&mut ask("the secret is out")) {
        Decision::Rejected(rejection) => assert_eq!(rejection.status, 400),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn text_nothing_matches_is_left_exactly_as_it_was() {
    let guard = guard(
        vec![rule(GuardAction::Mask, vec![pattern("secret")])],
        Vec::new(),
    );
    let mut body = ask("nothing to see");
    assert!(matches!(guard.check_request(&mut body), Decision::Allowed));
    assert_eq!(content(&body), "nothing to see");
}

#[test]
fn a_mask_rule_rewrites_and_lets_the_request_through() {
    let guard = guard(
        vec![rule(GuardAction::Mask, vec![pattern(r"hunter\d+")])],
        Vec::new(),
    );
    let mut body = ask("my password is hunter2, do not tell");
    assert!(matches!(guard.check_request(&mut body), Decision::Masked));
    assert_eq!(content(&body), "my password is <masked>, do not tell");
}

#[test]
fn a_builtin_says_what_it_found_and_a_custom_pattern_cannot() {
    let guard = guard(
        vec![rule(
            GuardAction::Mask,
            vec![GuardPattern::Builtin(Builtin::Email), pattern(r"ID-\d+")],
        )],
        Vec::new(),
    );
    let mut body = ask("write to a.b@example.com about ID-77");
    guard.check_request(&mut body);
    assert_eq!(content(&body), "write to <EMAIL> about <masked>");
}

#[test]
fn every_builtin_matches_the_shape_it_names() {
    let cases = [
        (
            Builtin::Email,
            "reach me at first.last+tag@example.co.uk",
            "<EMAIL>",
        ),
        (
            Builtin::PhoneNumber,
            "call 555-867-5309 today",
            "<PHONE_NUMBER>",
        ),
        (Builtin::Ssn, "ssn 123-45-6789 here", "<SSN>"),
        (
            Builtin::CreditCard,
            "card 4111 1111 1111 1111 ok",
            "<CREDIT_CARD>",
        ),
        (Builtin::CaSin, "sin 046 454 286 ok", "<CA_SIN>"),
    ];
    for (kind, text, token) in cases {
        let guard = guard(
            vec![rule(GuardAction::Mask, vec![GuardPattern::Builtin(kind)])],
            Vec::new(),
        );
        let mut body = ask(text);
        guard.check_request(&mut body);
        assert!(
            content(&body).contains(token),
            "{kind:?} should have matched `{text}`, got `{}`",
            content(&body)
        );
    }
}

#[test]
fn every_message_in_the_conversation_is_scanned_not_just_the_last() {
    // A conversation carries its own history, and a credential three turns
    // back is still on its way to the provider.
    let guard = guard(
        vec![rule(GuardAction::Mask, vec![pattern("hunter2")])],
        Vec::new(),
    );
    let mut body = json!({"messages": [
        {"role": "user", "content": "my password is hunter2"},
        {"role": "assistant", "content": "noted"},
        {"role": "user", "content": "what did I say?"},
    ]});
    guard.check_request(&mut body);
    assert_eq!(body["messages"][0]["content"], "my password is <masked>");
}

#[test]
fn the_first_rule_to_refuse_ends_it() {
    // So an operator can read a list top to bottom and know which refusal a
    // request will get.
    let mut first = rule(GuardAction::Reject, vec![pattern("alpha")]);
    first.rejection = Some(agentgateway_config::Rejection {
        status: 401,
        headers: None,
        body: None,
    });
    let mut second = rule(GuardAction::Reject, vec![pattern("beta")]);
    second.rejection = Some(agentgateway_config::Rejection {
        status: 402,
        headers: None,
        body: None,
    });
    let guard = guard(vec![first, second], Vec::new());

    match guard.check_request(&mut ask("alpha and beta")) {
        Decision::Rejected(rejection) => assert_eq!(rejection.status, 401),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_mask_rule_before_a_reject_rule_can_defuse_it() {
    // Order is the whole contract: the mask ran first, so by the time the
    // reject rule looked, the text it would have refused was gone.
    let guard = guard(
        vec![
            rule(GuardAction::Mask, vec![pattern("hunter2")]),
            rule(GuardAction::Reject, vec![pattern("hunter2")]),
        ],
        Vec::new(),
    );
    let mut body = ask("password hunter2");
    assert!(matches!(guard.check_request(&mut body), Decision::Masked));
    assert_eq!(content(&body), "password <masked>");
}

#[test]
fn a_structured_content_list_is_left_alone_rather_than_half_scanned() {
    // The multimodal shape: its text parts are reachable but its image parts
    // are not text at all.
    let guard = guard(
        vec![rule(GuardAction::Mask, vec![pattern("secret")])],
        Vec::new(),
    );
    let mut body = json!({"messages": [{"role": "user", "content": [
        {"type": "text", "text": "the secret"},
    ]}]});
    guard.check_request(&mut body);
    assert_eq!(body["messages"][0]["content"][0]["text"], "the secret");
}

#[test]
fn response_rules_run_on_the_answers_own_text() {
    let guard = guard(
        Vec::new(),
        vec![rule(
            GuardAction::Mask,
            vec![GuardPattern::Builtin(Builtin::PhoneNumber)],
        )],
    );
    let mut text = "you can call 555-867-5309".to_string();
    assert!(matches!(guard.check_text(&mut text), Decision::Masked));
    assert_eq!(text, "you can call <PHONE_NUMBER>");
}

#[test]
fn a_response_rule_is_what_makes_a_stream_buffer() {
    let guarded = guard(
        Vec::new(),
        vec![rule(GuardAction::Mask, vec![pattern("x")])],
    );
    assert!(guarded.guards_response());

    let request_only = guard(
        vec![rule(GuardAction::Mask, vec![pattern("x")])],
        Vec::new(),
    );
    assert!(
        !request_only.guards_response(),
        "a request rule costs a stream nothing"
    );
}

#[test]
fn a_pattern_that_does_not_compile_fails_at_startup() {
    // The alternative is a rule that silently never fires, which reads exactly
    // like content nobody sent.
    let err = Guard::new(
        Some(&PromptGuard {
            request: vec![rule(GuardAction::Reject, vec![pattern("[")])],
            response: Vec::new(),
        }),
        "route[0].ai.promptGuard",
    )
    .expect_err("should not compile");
    assert!(err.to_string().contains("route[0]"), "got: {err}");
    assert!(err.to_string().contains('['), "got: {err}");
}

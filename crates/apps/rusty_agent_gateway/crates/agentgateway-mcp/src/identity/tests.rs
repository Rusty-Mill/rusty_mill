#![allow(clippy::unwrap_used)]

use super::*;

fn config(strip: &[(&str, &[&str])], set: &[(&str, &str)], headers: &[(&str, &str)]) -> Identity {
    let owned = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    };
    Identity::new(
        &TargetIdentity {
            strip: strip
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.iter().map(|s| (*s).to_owned()).collect()))
                .collect(),
            set: owned(set),
            headers: owned(headers),
        },
        "t",
    )
    .unwrap()
}

fn board() -> Identity {
    config(
        &[("*", &["actor", "role", "agent_id", "agent_key"])],
        &[("actor", "lel-gateway/{{jwt.lel.lens}}-{{jwt.lel.claim:8}}")],
        &[("x-lel-sub", "{{jwt.sub}}")],
    )
}

fn claims() -> serde_json::Value {
    serde_json::json!({
        "sub": "rusty_mill/c-0123456789/builder-rust/1",
        "lel.lens": "builder-rust",
        "lel.claim": "c-0123456789",
    })
}

fn call(args: &str) -> CallToolParams {
    let mut params = CallToolParams::new("card_comment");
    params.arguments = Some(Value::from_json_str(args).unwrap());
    params
}

fn arg<'a>(params: &'a CallToolParams, key: &str) -> Option<&'a str> {
    params.arguments.as_ref()?.get(key)?.as_str()
}

#[test]
fn a_forged_actor_is_replaced_by_the_tokens() {
    let mut params = call(r#"{"id":"42","text":"hi","actor":"nano","agent_key":"k"}"#);
    let mut headers = HeaderOverride::default();
    board()
        .apply(&mut params, &mut headers, Some(&claims()))
        .unwrap();

    assert_eq!(
        arg(&params, "actor"),
        Some("lel-gateway/builder-rust-c-012345")
    );
    assert_eq!(arg(&params, "agent_key"), None);
    assert_eq!(arg(&params, "text"), Some("hi"));
    assert_eq!(headers.set.len(), 1);
    assert_eq!(headers.set[0].0.as_str(), "x-lel-sub");
    assert_eq!(headers.set[0].1, "rusty_mill/c-0123456789/builder-rust/1");
}

#[test]
fn no_token_refuses_the_call_and_changes_nothing() {
    let mut params = call(r#"{"actor":"nano"}"#);
    let mut headers = HeaderOverride::default();
    let err = board().apply(&mut params, &mut headers, None).unwrap_err();
    assert!(matches!(err, IdentityError::Unresolved { .. }));
    assert_eq!(arg(&params, "actor"), Some("nano"));
    assert!(headers.is_empty());
}

#[test]
fn a_missing_empty_or_structured_claim_refuses() {
    let identity = config(&[], &[("actor", "{{jwt.lel.lens}}")], &[]);
    for claims in [
        serde_json::json!({}),
        serde_json::json!({"lel.lens": ""}),
        serde_json::json!({"lel.lens": ["a"]}),
    ] {
        let err = identity
            .apply(
                &mut call("{}"),
                &mut HeaderOverride::default(),
                Some(&claims),
            )
            .unwrap_err();
        assert_eq!(
            err,
            IdentityError::Unresolved {
                claim: "lel.lens".to_owned()
            }
        );
    }
}

#[test]
fn a_number_claim_renders_as_text() {
    let identity = config(&[], &[("who", "u-{{jwt.uid}}")], &[]);
    let mut params = call("{}");
    identity
        .apply(
            &mut params,
            &mut HeaderOverride::default(),
            Some(&serde_json::json!({"uid": 7})),
        )
        .unwrap();
    assert_eq!(arg(&params, "who"), Some("u-7"));
}

#[test]
fn strip_follows_the_tool_glob() {
    let identity = config(&[("board_*", &["actor"])], &[], &[]);
    let mut kept = call(r#"{"actor":"a"}"#);
    kept.name = "search".to_owned();
    identity
        .apply(&mut kept, &mut HeaderOverride::default(), None)
        .unwrap();
    assert_eq!(arg(&kept, "actor"), Some("a"));

    let mut dropped = call(r#"{"actor":"a"}"#);
    dropped.name = "board_card_get".to_owned();
    identity
        .apply(&mut dropped, &mut HeaderOverride::default(), None)
        .unwrap();
    assert_eq!(arg(&dropped, "actor"), None);
}

#[test]
fn set_creates_arguments_when_the_call_has_none() {
    let identity = config(&[], &[("actor", "{{jwt.sub}}")], &[]);
    let mut params = CallToolParams::new("t");
    identity
        .apply(&mut params, &mut HeaderOverride::default(), Some(&claims()))
        .unwrap();
    assert_eq!(
        arg(&params, "actor"),
        Some("rusty_mill/c-0123456789/builder-rust/1")
    );
}

#[test]
fn non_object_arguments_are_refused() {
    let identity = config(&[], &[("actor", "{{jwt.sub}}")], &[]);
    let err = identity
        .apply(
            &mut call("[1]"),
            &mut HeaderOverride::default(),
            Some(&claims()),
        )
        .unwrap_err();
    assert_eq!(err, IdentityError::Arguments);
}

#[test]
fn the_identity_header_wins_over_an_earlier_change() {
    let mut headers = HeaderOverride {
        set: vec![(
            HeaderName::from_static("x-lel-sub"),
            HeaderValue::from_static("forged"),
        )],
        remove: vec![HeaderName::from_static("x-lel-sub")],
    };
    board()
        .apply(&mut call("{}"), &mut headers, Some(&claims()))
        .unwrap();
    assert_eq!(headers.set.len(), 1);
    assert_ne!(headers.set[0].1, "forged");
    assert!(headers.remove.is_empty());
}

#[test]
fn bad_templates_are_refused_at_startup() {
    for (value, kind) in [
        ("fixed", "names no"),
        ("{{jwt.sub", "unclosed"),
        ("{{mcpGuardrails.x}}", "placeholder"),
        ("{{jwt.}}", "placeholder"),
        ("{{jwt.sub:0}}", "placeholder"),
        ("{{jwt.sub:x}}", "placeholder"),
    ] {
        let err = Identity::new(
            &TargetIdentity {
                set: [("actor".to_owned(), value.to_owned())].into(),
                ..TargetIdentity::default()
            },
            "t",
        )
        .unwrap_err();
        assert!(err.to_string().contains(kind), "{value}: {err}");
        assert!(err.to_string().contains("t.identity.set.actor"), "{err}");
    }
}

#[test]
fn a_bad_header_name_is_refused_at_startup() {
    let err = Identity::new(
        &TargetIdentity {
            headers: [("bad header".to_owned(), "{{jwt.sub}}".to_owned())].into(),
            ..TargetIdentity::default()
        },
        "t",
    )
    .unwrap_err();
    assert!(err.to_string().contains("header name"), "{err}");
}

#[test]
fn a_header_value_that_is_not_visible_ascii_refuses() {
    let identity = config(&[], &[], &[("x-lel-sub", "{{jwt.sub}}")]);
    let mut headers = HeaderOverride::default();
    let err = identity
        .apply(
            &mut call("{}"),
            &mut headers,
            Some(&serde_json::json!({"sub": "caf\u{e9}"})),
        )
        .unwrap_err();
    assert_eq!(
        err,
        IdentityError::Header {
            name: "x-lel-sub".to_owned()
        }
    );
    assert!(headers.is_empty(), "nothing is stamped");
}

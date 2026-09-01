//! Unit tests for header templating on the MCP upstream path.

use std::collections::BTreeMap;

use super::*;
use crate::guardrails::Annotations;

fn modifier(set: &[(&str, &str)], add: &[(&str, &str)], remove: &[&str]) -> HeaderModifier {
    let map = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    HeaderModifier {
        set: map(set),
        add: map(add),
        remove: remove.iter().map(|n| n.to_string()).collect(),
    }
}

fn compile(set: &[(&str, &str)], add: &[(&str, &str)], remove: &[&str]) -> Transform {
    Transform::new(&modifier(set, add, remove), "test").expect("should compile")
}

/// Annotations as a guardrail would have left them.
fn annotations(pairs: serde_json::Value) -> Annotations {
    crate::guardrails::Annotations::for_test(pairs)
}

fn applied(transform: &Transform, annotations: &Annotations) -> Vec<(String, String)> {
    let mut changes = HeaderOverride::default();
    transform.apply(&mut changes, annotations);
    let mut out: Vec<(String, String)> = changes
        .set
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
fn a_literal_value_passes_through_unchanged() {
    let transform = compile(&[("x-fixed", "yes")], &[], &[]);
    assert_eq!(
        applied(&transform, &Annotations::default()),
        vec![("x-fixed".to_string(), "yes".to_string())],
        "a value with no placeholder does not need a guardrail to have run"
    );
}

#[test]
fn a_placeholder_is_filled_from_the_guardrails_bag() {
    let transform = compile(
        &[("x-classification", "{{mcpGuardrails.classification}}")],
        &[],
        &[],
    );
    assert_eq!(
        applied(
            &transform,
            &annotations(serde_json::json!({"classification": "phishing"}))
        ),
        vec![("x-classification".to_string(), "phishing".to_string())]
    );
}

#[test]
fn text_around_a_placeholder_is_kept() {
    let transform = compile(&[("x-tag", "class={{mcpGuardrails.c}};v=1")], &[], &[]);
    assert_eq!(
        applied(&transform, &annotations(serde_json::json!({"c": "spam"}))),
        vec![("x-tag".to_string(), "class=spam;v=1".to_string())]
    );
}

#[test]
fn several_placeholders_in_one_value_all_resolve() {
    let transform = compile(
        &[("x-both", "{{mcpGuardrails.a}}/{{mcpGuardrails.b}}")],
        &[],
        &[],
    );
    assert_eq!(
        applied(
            &transform,
            &annotations(serde_json::json!({"a": "one", "b": "two"}))
        ),
        vec![("x-both".to_string(), "one/two".to_string())]
    );
}

#[test]
fn whitespace_inside_a_placeholder_is_ignored() {
    let transform = compile(&[("x-c", "{{ mcpGuardrails.classification }}")], &[], &[]);
    assert_eq!(
        applied(
            &transform,
            &annotations(serde_json::json!({"classification": "ok"}))
        ),
        vec![("x-c".to_string(), "ok".to_string())]
    );
}

#[test]
fn a_non_string_annotation_renders_as_json() {
    let transform = compile(
        &[
            ("x-score", "{{mcpGuardrails.score}}"),
            ("x-blocked", "{{mcpGuardrails.blocked}}"),
        ],
        &[],
        &[],
    );
    assert_eq!(
        applied(
            &transform,
            &annotations(serde_json::json!({"score": 0.75, "blocked": true}))
        ),
        vec![
            ("x-blocked".to_string(), "true".to_string()),
            ("x-score".to_string(), "0.75".to_string()),
        ],
        "and a string renders as itself rather than as a quoted JSON string"
    );
}

#[test]
fn an_unresolved_placeholder_drops_its_header() {
    // Rather than sending the template text upstream as though it were data.
    // A guardrail that did not run should read as "no classification".
    let transform = compile(&[("x-c", "{{mcpGuardrails.missing}}")], &[], &[]);
    assert!(applied(&transform, &Annotations::default()).is_empty());
    assert!(applied(&transform, &annotations(serde_json::json!({"other": "x"}))).is_empty());
}

#[test]
fn a_null_annotation_counts_as_absent() {
    let transform = compile(&[("x-c", "{{mcpGuardrails.c}}")], &[], &[]);
    assert!(applied(&transform, &annotations(serde_json::json!({"c": null}))).is_empty());
}

#[test]
fn one_unresolved_header_does_not_take_the_others_with_it() {
    let transform = compile(
        &[
            ("x-known", "{{mcpGuardrails.here}}"),
            ("x-unknown", "{{mcpGuardrails.absent}}"),
            ("x-literal", "always"),
        ],
        &[],
        &[],
    );
    assert_eq!(
        applied(&transform, &annotations(serde_json::json!({"here": "yes"}))),
        vec![
            ("x-known".to_string(), "yes".to_string()),
            ("x-literal".to_string(), "always".to_string()),
        ]
    );
}

#[test]
fn the_route_wins_over_a_guardrails_own_header_mutation() {
    // Route configuration is the operator's written intent; a processor's
    // header mutation is a runtime decision. Upstream's ordering too: the bag
    // exists so that *subsequent* filters can read it.
    let transform = compile(&[("x-user-id", "from-config")], &[], &[]);
    let mut changes = HeaderOverride {
        set: vec![(
            HeaderName::from_static("x-user-id"),
            HeaderValue::from_static("from-processor"),
        )],
        remove: Vec::new(),
    };
    transform.apply(&mut changes, &Annotations::default());

    assert_eq!(changes.set.len(), 1);
    assert_eq!(changes.set[0].1, "from-config");
}

#[test]
fn a_set_cancels_a_pending_remove() {
    let transform = compile(&[("x-tenant", "acme")], &[], &[]);
    let mut changes = HeaderOverride {
        set: Vec::new(),
        remove: vec![HeaderName::from_static("x-tenant")],
    };
    transform.apply(&mut changes, &Annotations::default());

    assert!(changes.remove.is_empty());
    assert_eq!(changes.set[0].1, "acme");
}

#[test]
fn remove_wins_over_the_modifiers_own_set() {
    // Gateway API order: `remove` is applied last and beats both.
    let transform = compile(&[("x-drop", "value")], &[], &["x-drop"]);
    let mut changes = HeaderOverride::default();
    transform.apply(&mut changes, &Annotations::default());

    assert!(changes.set.is_empty());
    assert_eq!(changes.remove.len(), 1);
}

#[test]
fn add_joins_rather_than_replacing_when_the_name_is_taken() {
    // One value per name crosses to the transport, so `add` cannot append the
    // way it does on the HTTP proxy path. A comma-separated field line is how
    // HTTP spells a list in one header.
    let transform = compile(&[("x-scope", "read")], &[("x-scope", "write")], &[]);
    assert_eq!(
        applied(&transform, &Annotations::default()),
        vec![("x-scope".to_string(), "read, write".to_string())]
    );
}

#[test]
fn add_behaves_as_set_when_the_name_is_free() {
    let transform = compile(&[], &[("x-only", "one")], &[]);
    assert_eq!(
        applied(&transform, &Annotations::default()),
        vec![("x-only".to_string(), "one".to_string())]
    );
}

#[test]
fn an_empty_modifier_changes_nothing() {
    let transform = compile(&[], &[], &[]);
    assert!(transform.is_empty());
    assert!(applied(&transform, &Annotations::default()).is_empty());
}

#[test]
fn a_placeholder_naming_something_other_than_the_bag_fails_at_startup() {
    // The alternative is a header that silently never resolves, which reads
    // exactly like a guardrail that never ran.
    let err = Transform::new(&modifier(&[("x-c", "{{jwt.sub}}")], &[], &[]), "route")
        .expect_err("should not compile");
    assert!(
        err.to_string().contains("mcpGuardrails.<key>"),
        "got: {err}"
    );
}

#[test]
fn an_unclosed_placeholder_fails_at_startup() {
    let err = Transform::new(
        &modifier(&[("x-c", "{{mcpGuardrails.c")], &[], &[]),
        "route",
    )
    .expect_err("should not compile");
    assert!(err.to_string().contains("unclosed"), "got: {err}");
}

#[test]
fn a_placeholder_with_no_key_fails_at_startup() {
    let err = Transform::new(
        &modifier(&[("x-c", "{{mcpGuardrails.}}")], &[], &[]),
        "route",
    )
    .expect_err("should not compile");
    assert!(err.to_string().contains("key is missing"), "got: {err}");
}

#[test]
fn a_header_name_http_rejects_fails_at_startup() {
    let err = Transform::new(&modifier(&[("not a name", "v")], &[], &[]), "route")
        .expect_err("should not compile");
    assert!(err.to_string().contains("header name"), "got: {err}");
}

#[test]
fn a_literal_value_http_rejects_fails_at_startup() {
    let err = Transform::new(&modifier(&[("x-c", "bad\nvalue")], &[], &[]), "route")
        .expect_err("should not compile");
    assert!(err.to_string().contains("header value"), "got: {err}");
}

#[test]
fn a_resolved_value_http_rejects_drops_its_header() {
    // Caught at startup for a literal, but a guardrail's value only exists at
    // runtime. Dropping beats failing the call over a bad annotation.
    let transform = compile(&[("x-c", "{{mcpGuardrails.c}}")], &[], &[]);
    assert!(
        applied(
            &transform,
            &annotations(serde_json::json!({"c": "bad\nvalue"}))
        )
        .is_empty()
    );
}

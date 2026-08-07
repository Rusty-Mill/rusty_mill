//! Unit tests for the processor chain.
//!
//! The chain is driven against a real gRPC server on a real socket rather than
//! a stubbed client, because most of what can go wrong here lives in the wire
//! encoding and in what happens when the server is not there.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use agentgateway_config::{FailureMode, HeaderFilter, McpGuardrails, Phase, Processor};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use super::*;
use wire::{HeaderMutation, Pass};

use crate::rules::Subject;

/// What a scripted processor should answer.
#[derive(Clone)]
enum Script {
    Pass,
    /// Replace the body with this JSON.
    Rewrite(&'static str),
    /// Refuse with this code and reason.
    Refuse(authorization_error::Code, &'static str),
    /// Never answer, so the caller's budget has to end the call.
    Hang,
    /// Pass, but ask for these header changes on the upstream call.
    Headers(Vec<(&'static str, &'static str)>, Vec<&'static str>),
}

/// What the processor was asked, recorded for assertions.
#[derive(Default, Clone)]
struct Seen {
    methods: Vec<String>,
    bodies: Vec<Option<Vec<u8>>>,
    backends: Vec<Vec<String>>,
    headers: Vec<Vec<(String, String)>>,
    metadata: Vec<Option<prost_types::Struct>>,
    calls: usize,
}

/// A gRPC `ExtMcp` server, hand-rolled over `tonic`'s unary plumbing.
///
/// Serving this by hand rather than through generated server code keeps
/// `protoc` out of the build for tests too, and it is 30 lines of dispatch.
async fn processor(script: Script) -> (String, Arc<Mutex<Seen>>, CancellationToken) {
    use axum::body::Body;
    use http_body_util::BodyExt;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("processor should bind");
    let addr: SocketAddr = listener.local_addr().expect("should have an address");
    let seen = Arc::new(Mutex::new(Seen::default()));
    let shutdown = CancellationToken::new();

    let recorder = Arc::clone(&seen);
    let stopping = shutdown.clone();

    let app = axum::Router::new().fallback(axum::routing::any(
        move |request: axum::extract::Request| {
            let script = script.clone();
            let recorder = Arc::clone(&recorder);
            async move {
                let path = request.uri().path().to_string();
                let body = request
                    .into_body()
                    .collect()
                    .await
                    .map(|b| b.to_bytes())
                    .unwrap_or_default();

                if matches!(script, Script::Hang) {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                }

                // gRPC frames a message as a 1-byte compression flag and a
                // 4-byte big-endian length, then the protobuf.
                let message = &body[5..];
                let (method, params, backends, headers, metadata) =
                    if path.ends_with("CheckRequest") {
                        let request = McpRequest::decode(message).expect("should decode");
                        // The MCP call's own headers travel inside the message, not
                        // as HTTP headers on the gRPC call that carries it.
                        let headers = request
                            .headers
                            .iter()
                            .map(|h| {
                                (
                                    h.key.clone(),
                                    String::from_utf8_lossy(&h.value).into_owned(),
                                )
                            })
                            .collect();
                        (
                            request.method,
                            request.mcp_request,
                            request.service_names,
                            headers,
                            request.metadata_context,
                        )
                    } else {
                        let request = McpResponse::decode(message).expect("should decode");
                        (
                            request.method,
                            Some(request.mcp_response),
                            request.service_names,
                            Vec::new(),
                            request.metadata_context,
                        )
                    };

                if let Ok(mut seen) = recorder.lock() {
                    seen.calls += 1;
                    seen.methods.push(method);
                    seen.bodies.push(params);
                    seen.backends.push(backends);
                    seen.headers.push(headers);
                    seen.metadata.push(metadata);
                }

                let payload = if path.ends_with("CheckRequest") {
                    McpRequestResult {
                        result: Some(match &script {
                            Script::Rewrite(body) => {
                                request_result::Result::Mutated(body.as_bytes().to_vec())
                            }
                            Script::Refuse(code, reason) => {
                                request_result::Result::Error(AuthorizationError {
                                    code: *code as i32,
                                    reason: (*reason).to_string(),
                                    mcp_error: None,
                                })
                            }
                            _ => request_result::Result::Pass(Pass {}),
                        }),
                        header_mutation: match &script {
                            Script::Headers(set, remove) => Some(HeaderMutation {
                                set: set
                                    .iter()
                                    .map(|(k, v)| McpHeader {
                                        key: (*k).to_string(),
                                        value: v.as_bytes().to_vec(),
                                    })
                                    .collect(),
                                remove: remove.iter().map(|n| (*n).to_string()).collect(),
                            }),
                            _ => None,
                        },
                        ..Default::default()
                    }
                    .encode_to_vec()
                } else {
                    McpResponseResult {
                        result: Some(match &script {
                            Script::Rewrite(body) => {
                                response_result::Result::Mutated(body.as_bytes().to_vec())
                            }
                            Script::Refuse(code, reason) => {
                                response_result::Result::Error(AuthorizationError {
                                    code: *code as i32,
                                    reason: (*reason).to_string(),
                                    mcp_error: None,
                                })
                            }
                            _ => response_result::Result::Pass(Pass {}),
                        }),
                    }
                    .encode_to_vec()
                };

                let mut framed = Vec::with_capacity(payload.len() + 5);
                framed.push(0);
                framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                framed.extend_from_slice(&payload);

                http::Response::builder()
                    .status(200)
                    .header("content-type", "application/grpc")
                    .header("grpc-status", "0")
                    .body(Body::from(framed))
                    .expect("response should build")
            }
        },
    ));

    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move { stopping.cancelled().await })
            .await;
    });

    (format!("{addr}"), seen, shutdown)
}

fn processor_config(host: &str, methods: &[(&str, Phase)]) -> Processor {
    Processor {
        methods: methods
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect::<BTreeMap<_, _>>(),
        host: Some(host.to_string()),
        ..Default::default()
    }
}

/// A call context with no token and no headers, for the common case.
fn call(method: &'static str) -> CallContext<'static> {
    static EMPTY: std::sync::LazyLock<HeaderMap> = std::sync::LazyLock::new(HeaderMap::new);
    CallContext {
        method,
        headers: &EMPTY,
        claims: None,
        subject: None,
        target: None,
    }
}

/// A call context that names what it is about.
fn about<'a>(method: &'a str, subject: Subject<'a>, target: &'a str) -> CallContext<'a> {
    static EMPTY: std::sync::LazyLock<HeaderMap> = std::sync::LazyLock::new(HeaderMap::new);
    CallContext {
        method,
        headers: &EMPTY,
        claims: None,
        subject: Some(subject),
        target: Some(target),
    }
}

fn chain(processors: Vec<Processor>) -> Guardrails {
    Guardrails::new(&McpGuardrails { processors }, "test").expect("should compile")
}

use prost::Message as _;

#[tokio::test]
async fn a_passing_processor_leaves_the_call_alone() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let chain = chain(vec![processor_config(
        &host,
        &[("tools/call", Phase::Full)],
    )]);

    let outcome = chain
        .check_request(
            call("tools/call"),
            &["alpha".into()],
            Some(br#"{"name":"echo"}"#),
        )
        .await
        .outcome;

    assert_eq!(outcome, Outcome::Pass);
    let seen = seen.lock().expect("lock");
    assert_eq!(seen.methods, vec!["tools/call"]);
    assert_eq!(seen.backends[0], vec!["alpha".to_string()]);
    assert_eq!(
        seen.bodies[0].as_deref(),
        Some(br#"{"name":"echo"}"#.as_slice()),
        "the processor should see the params it is being asked about"
    );
    stop.cancel();
}

#[tokio::test]
async fn a_processor_can_rewrite_the_request() {
    // The reason this is a guardrail rather than a gate: redacting an argument
    // is not something a yes/no answer can express.
    let (host, _, stop) = processor(Script::Rewrite(r#"{"name":"echo","args":{}}"#)).await;
    let chain = chain(vec![processor_config(
        &host,
        &[("tools/call", Phase::Request)],
    )]);

    let outcome = chain
        .check_request(
            call("tools/call"),
            &["alpha".into()],
            Some(br#"{"name":"echo","args":{"secret":"x"}}"#),
        )
        .await
        .outcome;

    assert_eq!(
        outcome,
        Outcome::Mutated(br#"{"name":"echo","args":{}}"#.to_vec())
    );
    stop.cancel();
}

#[tokio::test]
async fn a_refusal_carries_the_processors_reason_and_code() {
    let (host, _, stop) = processor(Script::Refuse(
        authorization_error::Code::PermissionDenied,
        "not in group",
    ))
    .await;
    let chain = chain(vec![processor_config(
        &host,
        &[("tools/call", Phase::Request)],
    )]);

    let outcome = chain
        .check_request(call("tools/call"), &["alpha".into()], Some(b"{}"))
        .await
        .outcome;

    assert_eq!(
        outcome,
        Outcome::Reject {
            code: PERMISSION_DENIED,
            message: "not in group".into(),
            data: None,
        }
    );
    stop.cancel();
}

#[tokio::test]
async fn a_chain_composes_rather_than_each_link_seeing_the_original() {
    // The second processor must see what the first produced. A chain where
    // every link sees the original input would let a validator approve a
    // payload that a redactor before it had already changed.
    let (redactor, _, stop_a) = processor(Script::Rewrite(r#"{"redacted":true}"#)).await;
    let (validator, seen, stop_b) = processor(Script::Pass).await;

    let chain = chain(vec![
        processor_config(&redactor, &[("tools/call", Phase::Request)]),
        processor_config(&validator, &[("tools/call", Phase::Request)]),
    ]);

    chain
        .check_request(
            call("tools/call"),
            &["alpha".into()],
            Some(br#"{"secret":"x"}"#),
        )
        .await;

    let seen = seen.lock().expect("lock");
    assert_eq!(
        seen.bodies[0].as_deref(),
        Some(br#"{"redacted":true}"#.as_slice()),
        "the second processor should see the first one's rewrite"
    );
    stop_a.cancel();
    stop_b.cancel();
}

#[tokio::test]
async fn the_first_refusal_ends_the_chain() {
    let (refuser, _, stop_a) = processor(Script::Refuse(
        authorization_error::Code::Invalid,
        "malformed",
    ))
    .await;
    let (second, seen, stop_b) = processor(Script::Pass).await;

    let chain = chain(vec![
        processor_config(&refuser, &[("tools/call", Phase::Request)]),
        processor_config(&second, &[("tools/call", Phase::Request)]),
    ]);

    let outcome = chain
        .check_request(call("tools/call"), &["alpha".into()], Some(b"{}"))
        .await
        .outcome;

    assert!(matches!(outcome, Outcome::Reject { .. }));
    assert_eq!(
        seen.lock().expect("lock").calls,
        0,
        "a processor after a refusal should not be consulted"
    );
    stop_a.cancel();
    stop_b.cancel();
}

#[tokio::test]
async fn a_processor_only_runs_at_the_phase_it_was_given() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let chain = chain(vec![processor_config(
        &host,
        &[("tools/call", Phase::Response)],
    )]);

    assert!(!chain.runs_request("tools/call"));
    assert!(chain.runs_response("tools/call"));

    chain
        .check_request(call("tools/call"), &["alpha".into()], Some(b"{}"))
        .await;
    assert_eq!(
        seen.lock().expect("lock").calls,
        0,
        "a response-phase processor must not see the request"
    );

    chain
        .check_response(call("tools/call"), &["alpha".into()], b"{}")
        .await;
    assert_eq!(seen.lock().expect("lock").calls, 1);
    stop.cancel();
}

#[tokio::test]
async fn a_method_matching_no_pattern_bypasses_the_processor() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let chain = chain(vec![processor_config(
        &host,
        &[("tools/call", Phase::Full)],
    )]);

    chain
        .check_response(call("tools/list"), &["alpha".into()], b"{}")
        .await;

    assert_eq!(seen.lock().expect("lock").calls, 0);
    stop.cancel();
}

#[tokio::test]
async fn a_rewrite_of_a_request_with_no_params_is_discarded() {
    // `tools/list` has nothing to rewrite on the way in. Upstream discards the
    // mutation rather than erroring, and filtering a listing is response-phase
    // work either way.
    let (host, _, stop) = processor(Script::Rewrite(r#"{"nonsense":true}"#)).await;
    let chain = chain(vec![processor_config(
        &host,
        &[("tools/list", Phase::Request)],
    )]);

    let outcome = chain
        .check_request(call("tools/list"), &["alpha".into()], None)
        .await
        .outcome;

    assert_eq!(outcome, Outcome::Pass);
    stop.cancel();
}

#[tokio::test]
async fn an_unreachable_processor_refuses_by_default() {
    // The decision the whole policy turns on: a policy service that is down
    // must not become an open door.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("should bind");
    let dead = listener.local_addr().expect("should have an address");
    drop(listener);

    let chain = chain(vec![processor_config(
        &dead.to_string(),
        &[("tools/call", Phase::Request)],
    )]);

    let outcome = chain
        .check_request(call("tools/call"), &["alpha".into()], Some(b"{}"))
        .await
        .outcome;

    match outcome {
        Outcome::Reject { code, .. } => assert_eq!(code, INTERNAL_ERROR),
        other => panic!("an unreachable processor must not pass: {other:?}"),
    }
}

#[tokio::test]
async fn failing_open_has_to_be_asked_for() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("should bind");
    let dead = listener.local_addr().expect("should have an address");
    drop(listener);

    let mut config = processor_config(&dead.to_string(), &[("tools/call", Phase::Request)]);
    config.failure_mode = Some(FailureMode::FailOpen);

    let outcome = chain(vec![config])
        .check_request(call("tools/call"), &["alpha".into()], Some(b"{}"))
        .await
        .outcome;

    assert_eq!(outcome, Outcome::Pass);
}

#[tokio::test]
async fn a_hanging_processor_hits_its_budget_and_refuses() {
    let (host, _, stop) = processor(Script::Hang).await;
    let mut config = processor_config(&host, &[("tools/call", Phase::Request)]);
    config.timeout = Some("200ms".parse().expect("should parse"));

    let started = std::time::Instant::now();
    let outcome = chain(vec![config])
        .check_request(call("tools/call"), &["alpha".into()], Some(b"{}"))
        .await
        .outcome;

    assert!(matches!(outcome, Outcome::Reject { .. }));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "the budget should fire long before the 30s processor; took {:?}",
        started.elapsed()
    );
    stop.cancel();
}

#[tokio::test]
async fn headers_are_forwarded_and_the_deny_list_wins() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let mut config = processor_config(&host, &[("tools/call", Phase::Request)]);
    config.request_headers = HeaderFilter {
        allowed: vec!["x-tenant".into(), "authorization".into()],
        disallowed: vec!["Authorization".into()],
    };

    let mut headers = HeaderMap::new();
    headers.insert("x-tenant", "acme".parse().expect("valid"));
    headers.insert("authorization", "Bearer t".parse().expect("valid"));
    headers.insert("cookie", "session=s".parse().expect("valid"));

    chain(vec![config])
        .check_request(
            CallContext {
                method: "tools/call",
                headers: &headers,
                claims: None,
                subject: None,
                target: None,
            },
            &["alpha".into()],
            Some(b"{}"),
        )
        .await;

    let seen = seen.lock().expect("lock");
    let sent = &seen.headers[0];
    let has = |name: &str| sent.iter().any(|(k, _)| k == name);
    assert!(has("x-tenant"), "an allow-listed header should be sent");
    assert!(
        !has("authorization"),
        "disallowed wins over allowed, case-insensitively"
    );
    assert!(
        !has("cookie"),
        "a header off the allow-list should not be sent"
    );
    stop.cancel();
}

#[tokio::test]
async fn an_empty_allow_list_forwards_everything() {
    // The opposite of `extAuthz.includeHeaders`, whose empty list forwards
    // nothing. The difference is upstream's, and it is why this has a test.
    let (host, seen, stop) = processor(Script::Pass).await;
    let config = processor_config(&host, &[("tools/call", Phase::Request)]);

    let mut headers = HeaderMap::new();
    headers.insert("x-tenant", "acme".parse().expect("valid"));

    chain(vec![config])
        .check_request(
            CallContext {
                method: "tools/call",
                headers: &headers,
                claims: None,
                subject: None,
                target: None,
            },
            &["alpha".into()],
            Some(b"{}"),
        )
        .await;

    assert!(
        seen.lock().expect("lock").headers[0]
            .iter()
            .any(|(k, _)| k == "x-tenant")
    );
    stop.cancel();
}

#[test]
fn a_processor_naming_a_backend_is_dropped_and_reported() {
    // This build has no backend registry to resolve the name against. Dropping
    // it silently would leave an operator believing a guardrail was running.
    let config = McpGuardrails {
        processors: vec![Processor {
            methods: [("tools/call".to_string(), Phase::Full)].into(),
            backend: Some("policy-service".into()),
            ..Default::default()
        }],
    };

    let chain = Guardrails::new(&config, "test").expect("should compile");
    assert!(chain.is_empty());
    assert!(!chain.runs_request("tools/call"));
}

#[test]
fn a_host_that_is_not_an_address_fails_at_startup() {
    let config = McpGuardrails {
        processors: vec![Processor {
            methods: [("tools/call".to_string(), Phase::Full)].into(),
            host: Some("not a host".into()),
            ..Default::default()
        }],
    };

    let err = Guardrails::new(&config, "binds[0]").expect_err("should not compile");
    assert!(
        err.to_string().contains("binds[0].processors[0]"),
        "got: {err}"
    );
}

#[tokio::test]
async fn metadata_expressions_reach_the_processor() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let mut config = processor_config(&host, &[("tools/call", Phase::Request)]);
    config.metadata = [
        (
            "tenant".to_string(),
            r#"request.headers["x-tenant"]"#.to_string(),
        ),
        ("subject".to_string(), "jwt.sub".to_string()),
    ]
    .into();

    let mut headers = HeaderMap::new();
    headers.insert("x-tenant", "acme".parse().expect("valid"));
    let claims = serde_json::json!({"sub": "u-1"});

    chain(vec![config])
        .check_request(
            CallContext {
                method: "tools/call",
                headers: &headers,
                claims: Some(&claims),
                subject: None,
                target: None,
            },
            &["alpha".into()],
            Some(b"{}"),
        )
        .await;

    let seen = seen.lock().expect("lock");
    let fields = seen.metadata[0]
        .as_ref()
        .expect("metadata should be sent")
        .fields
        .clone();
    let text = |key: &str| match fields.get(key).and_then(|v| v.kind.clone()) {
        Some(prost_types::value::Kind::StringValue(s)) => s,
        other => panic!("{key} should be a string, got {other:?}"),
    };
    assert_eq!(text("tenant"), "acme");
    assert_eq!(text("subject"), "u-1");
    stop.cancel();
}

#[tokio::test]
async fn a_metadata_expression_that_cannot_be_evaluated_is_skipped_not_fatal() {
    // Metadata is context, not a decision. A missing claim on an
    // unauthenticated route should not take the guardrail offline; the
    // processor sees the key absent and decides for itself.
    let (host, seen, stop) = processor(Script::Pass).await;
    let mut config = processor_config(&host, &[("tools/call", Phase::Request)]);
    config.metadata = [
        ("subject".to_string(), "jwt.sub".to_string()),
        ("method".to_string(), "request.method".to_string()),
    ]
    .into();

    let outcome = chain(vec![config])
        .check_request(call("tools/call"), &["alpha".into()], Some(b"{}"))
        .await
        .outcome;

    assert_eq!(outcome, Outcome::Pass);
    let seen = seen.lock().expect("lock");
    let fields = &seen.metadata[0]
        .as_ref()
        .expect("metadata should be sent")
        .fields;
    assert!(
        !fields.contains_key("subject"),
        "an unevaluable key is dropped"
    );
    assert!(fields.contains_key("method"), "the others still arrive");
    stop.cancel();
}

#[tokio::test]
async fn no_metadata_configured_sends_no_metadata() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let config = processor_config(&host, &[("tools/call", Phase::Request)]);

    chain(vec![config])
        .check_request(call("tools/call"), &["alpha".into()], Some(b"{}"))
        .await;

    assert_eq!(seen.lock().expect("lock").metadata[0], None);
    stop.cancel();
}

#[test]
fn a_metadata_expression_that_does_not_compile_fails_at_startup() {
    let mut processor = Processor {
        methods: [("tools/call".to_string(), Phase::Full)].into(),
        host: Some("127.0.0.1:9000".into()),
        ..Default::default()
    };
    processor.metadata = [("tenant".to_string(), "jwt.sub ==".to_string())].into();

    let err = Guardrails::new(
        &McpGuardrails {
            processors: vec![processor],
        },
        "binds[0]",
    )
    .expect_err("should not compile");
    assert!(err.to_string().contains("metadata.tenant"), "got: {err}");
}

/// One processor's answer, folded into a running set.
fn changes(set: &[(&str, &str)], remove: &[&str]) -> HeaderChanges {
    let mut changes = HeaderChanges::default();
    changes.merge(HeaderMutation {
        set: set
            .iter()
            .map(|(k, v)| McpHeader {
                key: (*k).to_string(),
                value: v.as_bytes().to_vec(),
            })
            .collect(),
        remove: remove.iter().map(|n| (*n).to_string()).collect(),
    });
    changes
}

fn set_of(changes: &HeaderChanges) -> Vec<(String, String)> {
    changes
        .set()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

#[test]
fn a_set_and_a_remove_survive_the_merge() {
    let changes = changes(&[("x-user-id", "u-1")], &["x-internal"]);
    assert_eq!(set_of(&changes), vec![("x-user-id".into(), "u-1".into())]);
    assert_eq!(
        changes
            .remove()
            .iter()
            .map(|n| n.as_str())
            .collect::<Vec<_>>(),
        vec!["x-internal"]
    );
}

#[test]
fn repeated_set_entries_for_one_name_form_a_list() {
    // The protocol says they replace the existing header as a list, and a
    // comma-separated field line is how HTTP spells that.
    let changes = changes(&[("x-scope", "read"), ("x-scope", "write")], &[]);
    assert_eq!(
        set_of(&changes),
        vec![("x-scope".into(), "read, write".into())]
    );
}

#[test]
fn a_later_processor_overwrites_an_earlier_one() {
    // Each processor's mutation is applied in turn to the same map, so the
    // first entry of a later one overwrites rather than appending to what a
    // previous processor left behind.
    let mut changes = changes(&[("x-user-id", "u-1")], &[]);
    changes.merge(HeaderMutation {
        set: vec![McpHeader {
            key: "x-user-id".into(),
            value: b"u-2".to_vec(),
        }],
        remove: Vec::new(),
    });
    assert_eq!(set_of(&changes), vec![("x-user-id".into(), "u-2".into())]);
}

#[test]
fn a_later_remove_cancels_an_earlier_set_and_the_other_way_round() {
    let mut changes = changes(&[("x-user-id", "u-1")], &[]);
    changes.merge(HeaderMutation {
        set: Vec::new(),
        remove: vec!["x-user-id".into()],
    });
    assert!(changes.set().is_empty());
    assert_eq!(changes.remove().len(), 1);

    changes.merge(HeaderMutation {
        set: vec![McpHeader {
            key: "x-user-id".into(),
            value: b"u-3".to_vec(),
        }],
        remove: Vec::new(),
    });
    assert_eq!(set_of(&changes), vec![("x-user-id".into(), "u-3".into())]);
    assert!(changes.remove().is_empty());
}

#[test]
fn a_header_http_cannot_represent_is_skipped_not_fatal() {
    // A malformed header is a bug in the processor. Refusing every request
    // because of one is a worse outcome than dropping it and saying so.
    let changes = changes(&[("not a header", "x"), ("x-fine", "yes")], &["also bad"]);
    assert_eq!(set_of(&changes), vec![("x-fine".into(), "yes".into())]);
    assert!(changes.remove().is_empty());
}

#[test]
fn a_skipped_first_entry_does_not_turn_a_later_one_into_an_append() {
    // The value must be `u-2`, not `, u-2`: `written` is marked only after a
    // successful write.
    let changes = changes(&[("x-user-id", "u-\u{7f}bad"), ("x-user-id", "u-2")], &[]);
    assert_eq!(set_of(&changes), vec![("x-user-id".into(), "u-2".into())]);
}

#[tokio::test]
async fn a_header_mutation_reaches_the_caller_of_the_chain() {
    let (host, _, stop) = processor(Script::Headers(vec![("x-user-id", "u-42")], vec![])).await;
    let chain = chain(vec![processor_config(
        &host,
        &[("tools/call", Phase::Request)],
    )]);

    let decision = chain
        .check_request(call("tools/call"), &["alpha".into()], Some(b"{}"))
        .await;

    assert_eq!(decision.outcome, Outcome::Pass);
    assert_eq!(
        set_of(&decision.headers),
        vec![("x-user-id".to_string(), "u-42".to_string())]
    );
    stop.cancel();
}

#[tokio::test]
async fn a_refusal_carries_no_header_changes() {
    // A request that never happens should leave no trace on the one that
    // replaces it.
    let (setter, _, stop_a) = processor(Script::Headers(vec![("x-user-id", "u-42")], vec![])).await;
    let (refuser, _, stop_b) = processor(Script::Refuse(
        authorization_error::Code::PermissionDenied,
        "no",
    ))
    .await;

    let chain = chain(vec![
        processor_config(&setter, &[("tools/call", Phase::Request)]),
        processor_config(&refuser, &[("tools/call", Phase::Request)]),
    ]);

    let decision = chain
        .check_request(call("tools/call"), &["alpha".into()], Some(b"{}"))
        .await;

    assert!(matches!(decision.outcome, Outcome::Reject { .. }));
    assert!(decision.headers.is_empty());
    stop_a.cancel();
    stop_b.cancel();
}

#[tokio::test]
async fn header_changes_from_several_processors_accumulate() {
    let (first, _, stop_a) = processor(Script::Headers(vec![("x-user-id", "u-1")], vec![])).await;
    let (second, _, stop_b) = processor(Script::Headers(vec![("x-tenant", "acme")], vec![])).await;

    let chain = chain(vec![
        processor_config(&first, &[("tools/call", Phase::Request)]),
        processor_config(&second, &[("tools/call", Phase::Request)]),
    ]);

    let decision = chain
        .check_request(call("tools/call"), &["alpha".into()], Some(b"{}"))
        .await;

    let mut set = set_of(&decision.headers);
    set.sort();
    assert_eq!(
        set,
        vec![
            ("x-tenant".to_string(), "acme".to_string()),
            ("x-user-id".to_string(), "u-1".to_string()),
        ]
    );
    stop_a.cancel();
    stop_b.cancel();
}

/// The string value of a metadata key the processor was sent, if any.
fn metadata_text(seen: &Seen, key: &str) -> Option<String> {
    match seen.metadata[0].as_ref()?.fields.get(key)?.kind.clone()? {
        prost_types::value::Kind::StringValue(s) => Some(s),
        other => panic!("{key} should be a string, got {other:?}"),
    }
}

#[tokio::test]
async fn metadata_can_name_the_prompt_a_call_is_about() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let mut config = processor_config(&host, &[("prompts/get", Phase::Request)]);
    config.metadata = [
        ("prompt".to_string(), "mcp.prompt.name".to_string()),
        ("target".to_string(), "mcp.prompt.target".to_string()),
    ]
    .into();

    chain(vec![config])
        .check_request(
            about("prompts/get", Subject::Prompt("summarize"), "alpha"),
            &["alpha".into()],
            Some(b"{}"),
        )
        .await;

    let seen = seen.lock().expect("lock");
    assert_eq!(metadata_text(&seen, "prompt").as_deref(), Some("summarize"));
    assert_eq!(metadata_text(&seen, "target").as_deref(), Some("alpha"));
    stop.cancel();
}

#[tokio::test]
async fn metadata_can_name_the_resource_a_call_is_about() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let mut config = processor_config(&host, &[("resources/read", Phase::Request)]);
    config.metadata = [("uri".to_string(), "mcp.resource.name".to_string())].into();

    chain(vec![config])
        .check_request(
            about(
                "resources/read",
                Subject::Resource("memo:insights"),
                "alpha",
            ),
            &["alpha".into()],
            Some(b"{}"),
        )
        .await;

    assert_eq!(
        metadata_text(&seen.lock().expect("lock"), "uri").as_deref(),
        Some("memo:insights"),
        "the unmuxed URI, matching what the request body carries"
    );
    stop.cancel();
}

#[tokio::test]
async fn only_the_subjects_own_key_is_bound_in_metadata() {
    // The same rule `mcpAuthorization.rules` follows, and for the same reason:
    // `mcp.tool.name` on a prompt call must not quietly resolve to the prompt.
    let (host, seen, stop) = processor(Script::Pass).await;
    let mut config = processor_config(&host, &[("prompts/get", Phase::Request)]);
    config.metadata = [
        ("as_tool".to_string(), "mcp.tool.name".to_string()),
        ("as_prompt".to_string(), "mcp.prompt.name".to_string()),
    ]
    .into();

    chain(vec![config])
        .check_request(
            about("prompts/get", Subject::Prompt("summarize"), "alpha"),
            &["alpha".into()],
            Some(b"{}"),
        )
        .await;

    let seen = seen.lock().expect("lock");
    assert_eq!(
        metadata_text(&seen, "as_prompt").as_deref(),
        Some("summarize")
    );
    let fields = &seen.metadata[0].as_ref().expect("metadata").fields;
    assert!(
        !fields.contains_key("as_tool"),
        "a tool expression on a prompt call cannot evaluate, so its key is dropped"
    );
    stop.cancel();
}

#[tokio::test]
async fn a_listing_has_no_subject_to_name() {
    // `prompts/list` fans out across every target. There is no single prompt
    // it is about, so `mcp` is unbound and the key is dropped rather than
    // given something invented.
    let (host, seen, stop) = processor(Script::Pass).await;
    let mut config = processor_config(&host, &[("prompts/list", Phase::Request)]);
    config.metadata = [
        ("prompt".to_string(), "mcp.prompt.name".to_string()),
        ("method".to_string(), "request.method".to_string()),
    ]
    .into();

    let outcome = chain(vec![config])
        .check_request(call("prompts/list"), &["alpha".into(), "beta".into()], None)
        .await;

    assert_eq!(outcome.outcome, Outcome::Pass);
    let seen = seen.lock().expect("lock");
    let fields = &seen.metadata[0].as_ref().expect("metadata").fields;
    assert!(!fields.contains_key("prompt"));
    assert_eq!(
        metadata_text(&seen, "method").as_deref(),
        Some("prompts/list"),
        "the request context is still there"
    );
    stop.cancel();
}

#[tokio::test]
async fn metadata_mixes_the_subject_with_the_caller() {
    // The combination that makes this worth having: a processor told both who
    // is asking and what they are asking for, without parsing the body.
    let (host, seen, stop) = processor(Script::Pass).await;
    let mut config = processor_config(&host, &[("resources/read", Phase::Request)]);
    config.metadata = [(
        "who_wants_what".to_string(),
        r#"jwt.sub + " -> " + mcp.resource.name"#.to_string(),
    )]
    .into();

    let claims = serde_json::json!({"sub": "u-1"});
    let headers = HeaderMap::new();
    chain(vec![config])
        .check_request(
            CallContext {
                method: "resources/read",
                headers: &headers,
                claims: Some(&claims),
                subject: Some(Subject::Resource("memo:insights")),
                target: Some("alpha"),
            },
            &["alpha".into()],
            Some(b"{}"),
        )
        .await;

    assert_eq!(
        metadata_text(&seen.lock().expect("lock"), "who_wants_what").as_deref(),
        Some("u-1 -> memo:insights")
    );
    stop.cancel();
}

#[tokio::test]
async fn the_response_phase_gets_the_subject_too() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let mut config = processor_config(&host, &[("prompts/get", Phase::Response)]);
    config.metadata = [("prompt".to_string(), "mcp.prompt.name".to_string())].into();

    chain(vec![config])
        .check_response(
            about("prompts/get", Subject::Prompt("summarize"), "alpha"),
            &["alpha".into()],
            b"{}",
        )
        .await;

    assert_eq!(
        metadata_text(&seen.lock().expect("lock"), "prompt").as_deref(),
        Some("summarize")
    );
    stop.cancel();
}

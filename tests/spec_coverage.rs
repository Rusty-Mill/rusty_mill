//! The README's central claim, checked against the specification.
//!
//! > Every endpoint and schema in the ACP v0.2.0 OpenAPI document is implemented.
//!
//! That sentence and the table under it were maintained by hand, the document
//! was not in the repository, and no test read it — so nothing failed when the
//! two drifted. A checkable claim beside it already had drifted: the README
//! said 163 tests while the crate had grown to 295.
//!
//! `tests/types.rs` checks that what this crate serializes it can deserialize,
//! which is a real property and **not this one**. A field named consistently
//! but named *wrongly* round-trips perfectly and is invisible there.
//!
//! # Both directions
//!
//! Forward — every operation the specification declares is routed. That is the
//! claim the README makes.
//!
//! Reverse — every route served that the specification does *not* declare is
//! listed in [`EXTENSIONS`] with its reason. This is the half worth having.
//! Nine merged pull requests added surface ACP does not define, each argued for
//! where it landed, and the set had grown past what anyone should be expected
//! to remember. Turning it into data means a tenth cannot be added silently.

#![cfg(all(feature = "server", feature = "client"))]

use std::collections::BTreeSet;
use std::sync::Arc;

use reqwest::{Method, StatusCode};
use rusty_acp::server::store::InMemoryStore;
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{AgentManifest, AgentName};

/// The vendored document. See `spec/README.md` for why it is vendored rather
/// than fetched, and how to move to another version.
const SPEC: &str = include_str!("../spec/acp-0.2.0-openapi.yaml");

/// The version this crate targets. Asserted against the document's own
/// `info.version` so a file swapped underneath the test says so, rather than
/// quietly grading the crate against a different specification.
const TARGET_VERSION: &str = "0.2.0";

/// Routes this crate serves that ACP does not define, and why each exists.
///
/// Every entry was a deliberate decision made in the pull request that added
/// it. Listing them here is not bookkeeping: it is the only place the whole set
/// is visible at once, and `no_undeclared_extensions` fails if a route appears
/// that is in neither the specification nor this list.
const EXTENSIONS: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/ready",
        "Readiness for a load balancer, distinct from ACP's /ping liveness check: \
         a draining replica is live but must stop receiving work.",
    ),
    (
        "GET",
        "/session/{session_id}/messages/{index}",
        "Backs the history links ACP puts in a session. The spec says history is \
         a list of URLs on resource servers and leaves where they point to the \
         implementation; these are where this one points.",
    ),
    (
        "GET",
        "/session/{session_id}/state",
        "Backs Session.state, which ACP models as a link rather than an inline \
         document so GET /session/{id} stays small however large the state grows.",
    ),
    (
        "GET",
        "/.well-known/agent.yml",
        "Open discovery, behind the `well-known` feature. Serves the same \
         manifests as GET /agents, as YAML, to an unauthenticated crawler.",
    ),
];

/// One operation the specification declares.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Operation {
    method: String,
    path: String,
}

/// Pull `paths:` out of the document.
///
/// A hand-rolled walk rather than a full OpenAPI parser. The alternative was a
/// dependency whose job is to understand every construct in a specification
/// this reads two levels of, and a dev-dependency is not free — it has to be
/// audited, kept current, and carried by everyone running the suite. What is
/// needed here is the set of `path: method` pairs, and the document's shape at
/// that depth is fixed by OpenAPI itself.
fn declared_operations(document: &str) -> BTreeSet<Operation> {
    const VERBS: [&str; 5] = ["get", "post", "put", "delete", "patch"];

    let mut operations = BTreeSet::new();
    let mut in_paths = false;
    let mut path = String::new();

    for line in document.lines() {
        if line.starts_with("paths:") {
            in_paths = true;
            continue;
        }
        if !in_paths {
            continue;
        }
        // A non-indented line ends the block — the next top-level key.
        if !line.is_empty() && !line.starts_with(' ') {
            break;
        }

        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.strip_prefix("  /") {
            if let Some(name) = rest.strip_suffix(':') {
                path = format!("/{name}");
            }
        } else if let Some(rest) = trimmed.strip_prefix("    ") {
            if !rest.starts_with(' ') {
                if let Some(verb) = rest.strip_suffix(':') {
                    if VERBS.contains(&verb) && !path.is_empty() {
                        operations
                            .insert(Operation { method: verb.to_uppercase(), path: path.clone() });
                    }
                }
            }
        }
    }
    operations
}

/// The value of a top-level-ish scalar, for the version assertion.
fn scalar_under(document: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in document.lines() {
        if line.starts_with(&format!("{section}:")) {
            in_section = true;
            continue;
        }
        if in_section {
            if !line.is_empty() && !line.starts_with(' ') {
                return None;
            }
            if let Some(rest) = line.trim_end().strip_prefix(&format!("  {key}:")) {
                return Some(rest.trim().to_string());
            }
        }
    }
    None
}

/// A running server, and the base URL to probe it at.
async fn server() -> String {
    let echo = agent_fn(
        AgentManifest::new(AgentName::new("echo").unwrap(), "Echoes"),
        |ctx: RunContext| async move { ctx.reply_text(ctx.input_text()).await.map(|_| ()) },
    );
    let router = AcpServer::builder()
        .agent(echo)
        .store(Arc::new(InMemoryStore::default()))
        .base_url("http://acp.example")
        .build()
        .unwrap()
        .into_router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

/// Fill a templated path with values that are syntactically valid and refer to
/// nothing, so a reply can only be about routing.
fn concrete(path: &str) -> String {
    path.replace("{run_id}", "00000000-0000-4000-8000-000000000000")
        .replace("{session_id}", "00000000-0000-4000-8000-000000000001")
        .replace("{name}", "echo")
        .replace("{index}", "0")
}

/// Whether the server has a route for this operation.
///
/// The discriminator is exact rather than a guess, because axum and the
/// handlers answer differently:
///
/// - no route at all — `404` with an **empty** body, from the router's fallback
/// - route exists, wrong method — `405`
/// - route exists, resource absent — `404` carrying an ACP error object
///
/// So a 404 says nothing on its own, and "did the body come from a handler"
/// is what separates *unimplemented* from *not found*. Anything that is not a
/// bare 404 or a 405 means the request reached a handler, which is all this
/// asks.
async fn is_routed(base_url: &str, method: &str, path: &str) -> bool {
    let url = format!("{base_url}{}", concrete(path));
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let response = reqwest::Client::new()
        .request(method, &url)
        // An empty JSON body: enough for the extractor to run, not enough to
        // be valid input. A 400 or 422 back is still proof of a route.
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();

    if response.status() == StatusCode::METHOD_NOT_ALLOWED {
        return false;
    }
    if response.status() == StatusCode::NOT_FOUND {
        // A handler's 404 carries an ACP error; the router's carries nothing.
        let body = response.text().await.unwrap_or_default();
        return body.contains("\"code\"");
    }
    true
}

/// The claim the README makes, checked rather than asserted.
#[tokio::test]
async fn every_declared_operation_is_served() {
    let base_url = server().await;
    let declared = declared_operations(SPEC);
    assert!(!declared.is_empty(), "the vendored document parsed to nothing");

    let mut missing = Vec::new();
    for operation in &declared {
        if !is_routed(&base_url, &operation.method, &operation.path).await {
            missing.push(format!("{} {}", operation.method, operation.path));
        }
    }

    assert!(
        missing.is_empty(),
        "the README claims every endpoint in the specification is implemented; \
         these are not routed: {missing:?}"
    );
}

/// The other direction: nothing is served that is neither in the specification
/// nor declared as an extension.
///
/// The router is read from its own source rather than introspected, because
/// `axum::Router` exposes no route table — there is no API that answers "what
/// do you serve". Scraping the one function that builds it is the mechanical
/// option, and it is exact: those are literal strings in a single `.route()`
/// chain, so a route added without a line here fails this immediately.
#[test]
fn no_undeclared_extensions() {
    let source = include_str!("../src/server/routes.rs");
    let declared: BTreeSet<String> =
        declared_operations(SPEC).into_iter().map(|op| op.path).collect();
    let extensions: BTreeSet<&str> = EXTENSIONS.iter().map(|(_, path, _)| *path).collect();

    // Every occurrence in the line, not just one at the start: the
    // `well-known` route is added by a separate `let router = router.route(..)`
    // rather than in the main chain, and an anchored match would have skipped
    // exactly the kind of route this is looking for.
    let mut undeclared = Vec::new();
    for line in source.lines() {
        for occurrence in line.split(".route(\"").skip(1) {
            let Some(path) = occurrence.split('"').next() else { continue };
            if !declared.contains(path) && !extensions.contains(path) {
                undeclared.push(path.to_string());
            }
        }
    }

    assert!(
        undeclared.is_empty(),
        "these routes are in neither the specification nor EXTENSIONS — add them to \
         EXTENSIONS with the reason they exist: {undeclared:?}"
    );
}

/// Every extension is real, so the list cannot rot into fiction.
///
/// Without this the reverse check could be satisfied by listing anything at
/// all, and a route that was removed would leave a justification behind
/// explaining a thing the server no longer does.
#[tokio::test]
async fn every_declared_extension_is_served() {
    let base_url = server().await;

    for (method, path, reason) in EXTENSIONS {
        // `well-known` is off in this build; its route is genuinely absent.
        if !cfg!(feature = "well-known") && path.starts_with("/.well-known") {
            continue;
        }
        assert!(
            is_routed(&base_url, method, path).await,
            "EXTENSIONS lists {method} {path} — {reason} — but nothing serves it"
        );
    }
}

/// The vendored document is the version this crate says it targets.
///
/// Cheap, and it is the assertion that keeps every other one in this file
/// meaningful: a document swapped for a later revision would otherwise have the
/// suite quietly grading the crate against a specification nobody chose.
#[test]
fn the_vendored_spec_is_the_version_this_crate_targets() {
    let version =
        scalar_under(SPEC, "info", "version").expect("the document has no info.version to check");
    assert_eq!(
        version, TARGET_VERSION,
        "the vendored document is v{version} but this crate targets v{TARGET_VERSION}"
    );
}

/// The README's table names every operation the specification declares.
///
/// The table is the first thing a reader checks the claim against, and it was
/// the thing most able to drift — being prose, nothing broke when an endpoint
/// was added or renamed. Not generated, because generating it would mean a
/// build step and lose the hand-written client-method column; checked instead,
/// which costs nothing and catches the same omission.
#[test]
fn the_readme_table_covers_the_specification() {
    let readme = include_str!("../README.md");
    let table = readme
        .split_once("## Protocol coverage")
        .expect("the README has no protocol coverage section")
        .1;

    let mut absent = Vec::new();
    for operation in declared_operations(SPEC) {
        if !table.contains(&operation.path) {
            absent.push(format!("{} {}", operation.method, operation.path));
        }
    }

    assert!(absent.is_empty(), "the README's coverage table does not mention: {absent:?}");
}

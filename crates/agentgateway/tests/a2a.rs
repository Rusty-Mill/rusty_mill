//! End-to-end tests for the `a2a` policy.
//!
//! The backends are mock agents serving a real agent card at the well-known
//! path and echoing JSON-RPC calls, so the assertions are about what actually
//! reached the agent — and, for discovery, about what a client would read off
//! the card the gateway serves.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

/// A port nothing in this binary has been handed yet.
///
/// Binding to port 0 and dropping the listener leaves a window in which the
/// same port can be handed out twice, and two tests racing for it fail with
/// `Address already in use`. Remembering what has been issued closes the
/// window between tests, which is where the collisions actually came from.
async fn free_port() -> u16 {
    use std::collections::HashSet;
    use std::sync::{LazyLock, Mutex};
    static ISSUED: LazyLock<Mutex<HashSet<u16>>> = LazyLock::new(Default::default);

    for _ in 0..64 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("should bind");
        let port = listener.local_addr().expect("should have an addr").port();
        drop(listener);
        if ISSUED.lock().expect("lock").insert(port) {
            return port;
        }
    }
    panic!("could not find a port this binary has not already used");
}

/// An agent card as a real A2A agent would serve it.
fn agent_card(name: &str, url: &str, skills: &[&str], streaming: bool) -> Value {
    json!({
        "name": name,
        "description": format!("{name} agent"),
        "version": "1.0",
        "protocolVersion": "1.0",
        "supportedInterfaces": [
            {"protocolBinding": "JSONRPC", "protocolVersion": "1.0", "url": url}
        ],
        "capabilities": {"streaming": streaming},
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": skills.iter().map(|id| json!({
            "id": id, "name": id, "description": "a skill", "tags": []
        })).collect::<Vec<_>>(),
    })
}

struct Agent {
    port: u16,
    calls: Arc<Mutex<Vec<Value>>>,
    /// Headers the agent saw on the last forwarded call.
    seen_headers: Arc<Mutex<Vec<(String, String)>>>,
    /// The path the last forwarded call arrived on.
    seen_path: Arc<Mutex<String>>,
    hits: Arc<AtomicUsize>,
}

/// Start a mock agent. `card` is `None` to serve a malformed one.
async fn agent(
    name: &'static str,
    skills: &'static [&'static str],
    streaming: bool,
    valid_card: bool,
) -> Agent {
    use axum::{Router, extract::Request, routing::any};

    let port = free_port().await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen_headers = Arc::new(Mutex::new(Vec::new()));
    let seen_path = Arc::new(Mutex::new(String::new()));
    let hits = Arc::new(AtomicUsize::new(0));

    let recorder = Arc::clone(&calls);
    let header_recorder = Arc::clone(&seen_headers);
    let path_recorder = Arc::clone(&seen_path);
    let counter = Arc::clone(&hits);

    let app = Router::new().fallback(any(move |request: Request| {
        let recorder = Arc::clone(&recorder);
        let header_recorder = Arc::clone(&header_recorder);
        let path_recorder = Arc::clone(&path_recorder);
        let counter = Arc::clone(&counter);
        async move {
            let path = request.uri().path().to_string();
            let is_get = request.method() == axum::http::Method::GET;

            if is_get && path.ends_with("/.well-known/agent-card.json") {
                let body = if valid_card {
                    agent_card(name, &format!("http://127.0.0.1:{port}"), skills, streaming)
                        .to_string()
                } else {
                    // Missing every required field but `name`.
                    json!({"name": name}).to_string()
                };
                return axum::response::Response::builder()
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .expect("response should build");
            }

            counter.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut seen) = path_recorder.lock() {
                seen.clone_from(&path);
            }
            if let Ok(mut seen) = header_recorder.lock() {
                *seen = request
                    .headers()
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.as_str().to_string(),
                            v.to_str().unwrap_or_default().to_string(),
                        )
                    })
                    .collect();
            }
            let bytes = axum::body::to_bytes(request.into_body(), 1 << 20)
                .await
                .unwrap_or_default();
            if let Ok(call) = serde_json::from_slice::<Value>(&bytes)
                && let Ok(mut calls) = recorder.lock()
            {
                calls.push(call);
            }

            axum::response::Response::builder()
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": true}}).to_string(),
                ))
                .expect("response should build")
        }
    }));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("agent should bind");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Agent {
        port,
        calls,
        seen_headers,
        seen_path,
        hits,
    }
}

/// Boot a gateway with an `a2a` route in front of `agents`.
async fn start(policy: &str, agents: &[u16]) -> (String, CancellationToken) {
    start_with(policy, agents, "").await
}

/// The same, with extra route policies spliced in.
async fn start_with(
    policy: &str,
    agents: &[u16],
    extra_policies: &str,
) -> (String, CancellationToken) {
    start_at("", policy, agents, extra_policies).await
}

/// The same again, with the route matching on a `pathPrefix`.
///
/// A `prefix` rewrite needs one to anchor on, and a route with no `matches`
/// has none.
async fn start_at(
    prefix: &str,
    policy: &str,
    agents: &[u16],
    extra_policies: &str,
) -> (String, CancellationToken) {
    let port = free_port().await;
    let backends: String = agents
        .iter()
        .map(|p| format!("              - host: \"127.0.0.1:{p}\"\n"))
        .collect();
    let matches = match prefix {
        "" => String::new(),
        prefix => format!(
            "            matches:\n              - path:\n                  pathPrefix: {prefix}\n"
        ),
    };

    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - name: agents
{matches}            policies:
{extra_policies}
              a2a:
{policy}
            backends:
{backends}
"#
    );

    let config = Config::from_yaml(&yaml).expect("config should parse");
    config.validate().expect("config should validate");
    let gateway = Gateway::build(&config, None)
        .await
        .expect("gateway should build");

    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    let _serving = serve::run_with_shutdown(gateway, vec![addr], shutdown.clone())
        .await
        .expect("gateway should bind");

    (format!("http://127.0.0.1:{port}"), shutdown)
}

async fn call(url: &str, method: &str) -> Value {
    reqwest::Client::new()
        .post(url)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": {}}))
        .send()
        .await
        .expect("request should reach the gateway")
        .json()
        .await
        .expect("should be JSON")
}

const CARD_POLICY: &str = r#"                agentCard:
                  url: "https://gateway.example.com/a2a""#;

#[tokio::test]
async fn a_permitted_method_reaches_the_agent() {
    let a = agent("Alpha", &["echo"], true, true).await;
    let (base, shutdown) = start(
        "                denyMethods: [\"^tasks/cancel$\"]",
        &[a.port],
    )
    .await;

    let response = call(&base, "message/send").await;
    assert_eq!(response["result"]["ok"], true);
    assert_eq!(a.hits.load(Ordering::Relaxed), 1);

    let calls = a.calls.lock().expect("lock");
    assert_eq!(
        calls[0]["method"], "message/send",
        "the body arrives intact"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_denied_method_never_reaches_the_agent() {
    let a = agent("Alpha", &["echo"], true, true).await;
    let (base, shutdown) = start(
        "                denyMethods: [\"^tasks/cancel$\"]",
        &[a.port],
    )
    .await;

    let response = call(&base, "tasks/cancel").await;

    assert_eq!(
        response["error"]["code"], -32011,
        "the spec's PermissionDenied code, not an invented one"
    );
    assert_eq!(
        response["id"], 1,
        "a client matches the response to its call"
    );
    assert_eq!(
        a.hits.load(Ordering::Relaxed),
        0,
        "a refused method must not be forwarded"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn an_allow_list_excludes_what_it_does_not_name() {
    let a = agent("Alpha", &["echo"], true, true).await;
    let (base, shutdown) = start("                allowMethods: [\"^message/\"]", &[a.port]).await;

    assert_eq!(call(&base, "message/send").await["result"]["ok"], true);
    assert_eq!(call(&base, "tasks/get").await["error"]["code"], -32011);
    assert_eq!(a.hits.load(Ordering::Relaxed), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn a_non_json_rpc_body_is_passed_through() {
    // A2A also has REST and gRPC bindings; refusing what the gate cannot read
    // would break them for no security benefit.
    let a = agent("Alpha", &["echo"], true, true).await;
    let (base, shutdown) = start("                denyMethods: [\"^tasks/\"]", &[a.port]).await;

    let response = reqwest::Client::new()
        .post(&base)
        .json(&json!({"message": {"role": "user", "parts": []}}))
        .send()
        .await
        .expect("should reach the gateway");

    assert_eq!(response.status(), 200);
    assert_eq!(
        a.hits.load(Ordering::Relaxed),
        1,
        "a REST-shaped body should still reach the agent"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn the_served_card_names_the_gateway_not_the_agent() {
    // The whole point of proxying a card: a client that reads the agent's own
    // URL goes around the gateway, past its auth and its audit trail.
    let a = agent("Alpha", &["echo"], true, true).await;
    let (base, shutdown) = start(CARD_POLICY, &[a.port]).await;

    let card: Value = reqwest::get(format!("{base}/.well-known/agent-card.json"))
        .await
        .expect("should reach the gateway")
        .json()
        .await
        .expect("should be JSON");

    assert_eq!(
        card["supportedInterfaces"][0]["url"], "https://gateway.example.com/a2a",
        "got: {card}"
    );
    assert_eq!(card["name"], "Alpha", "otherwise a faithful passthrough");

    shutdown.cancel();
}

#[tokio::test]
async fn skills_from_several_agents_are_unioned() {
    let a = agent("Alpha", &["echo"], true, true).await;
    let b = agent("Beta", &["summarise"], true, true).await;
    let (base, shutdown) = start(CARD_POLICY, &[a.port, b.port]).await;

    let card: Value = reqwest::get(format!("{base}/.well-known/agent-card.json"))
        .await
        .expect("should reach the gateway")
        .json()
        .await
        .expect("should be JSON");

    let ids: Vec<&str> = card["skills"]
        .as_array()
        .expect("skills")
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    assert_eq!(ids, vec!["echo", "summarise"]);

    shutdown.cancel();
}

#[tokio::test]
async fn capabilities_are_intersected_across_agents() {
    // Advertising streaming because one agent has it sends a streaming client
    // to an agent that cannot, and the failure lands on the client.
    let a = agent("Alpha", &["echo"], true, true).await;
    let b = agent("Beta", &["summarise"], false, true).await;
    let (base, shutdown) = start(CARD_POLICY, &[a.port, b.port]).await;

    let card: Value = reqwest::get(format!("{base}/.well-known/agent-card.json"))
        .await
        .expect("should reach the gateway")
        .json()
        .await
        .expect("should be JSON");

    assert_eq!(card["capabilities"]["streaming"], false, "got: {card}");

    shutdown.cancel();
}

#[tokio::test]
async fn one_malformed_card_does_not_break_discovery_for_the_rest() {
    // rusty_a2a's types are strict by design. A gateway aggregating cards from
    // agents it does not control has to be liberal about it.
    let good = agent("Alpha", &["echo"], true, true).await;
    let bad = agent("Broken", &["nope"], true, false).await;
    let (base, shutdown) = start(CARD_POLICY, &[good.port, bad.port]).await;

    let response = reqwest::get(format!("{base}/.well-known/agent-card.json"))
        .await
        .expect("should reach the gateway");
    assert_eq!(response.status(), 200);

    let card: Value = response.json().await.expect("should be JSON");
    assert_eq!(card["name"], "Alpha");
    let ids: Vec<&str> = card["skills"]
        .as_array()
        .expect("skills")
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    assert_eq!(ids, vec!["echo"], "the broken agent is simply absent");

    shutdown.cancel();
}

#[tokio::test]
async fn a_route_without_a_card_policy_forwards_discovery_to_the_agent() {
    // No `agentCard` means the gateway has no opinion, so the request is an
    // ordinary proxied GET.
    let a = agent("Alpha", &["echo"], true, true).await;
    let (base, shutdown) = start("                denyMethods: []", &[a.port]).await;

    let card: Value = reqwest::get(format!("{base}/.well-known/agent-card.json"))
        .await
        .expect("should reach the gateway")
        .json()
        .await
        .expect("should be JSON");

    assert_eq!(
        card["supportedInterfaces"][0]["url"],
        format!("http://127.0.0.1:{}", a.port),
        "the agent's own card, unmodified"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn every_agent_card_being_unusable_is_a_503_not_a_broken_card() {
    let bad = agent("Broken", &["nope"], true, false).await;
    let (base, shutdown) = start(CARD_POLICY, &[bad.port]).await;

    let response = reqwest::get(format!("{base}/.well-known/agent-card.json"))
        .await
        .expect("should reach the gateway");
    assert_eq!(
        response.status(),
        503,
        "serving a half-built card would be worse than admitting there is none"
    );

    shutdown.cancel();
}

/// The response headers the gateway returned for one request.
async fn headers_of(response: reqwest::Response) -> Vec<(String, String)> {
    response
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                v.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

/// A route whose `responseHeaderModifier` stamps every response.
const STAMP: &str = "              responseHeaderModifier:\n                set:\n                  x-served-by: rusty\n                remove: [x-agent]";

#[tokio::test]
async fn a_response_modifier_reaches_a_proxied_a2a_response() {
    let agent = agent("Alpha", &["echo"], true, true).await;
    let (url, shutdown) = start_with(
        "                allowMethods: [\"message/send\"]",
        &[agent.port],
        STAMP,
    )
    .await;

    let response = reqwest::Client::new()
        .post(&url)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "message/send", "params": {}}))
        .send()
        .await
        .expect("the gateway should answer");
    let headers = headers_of(response).await;

    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "x-served-by" && v == "rusty"),
        "saw {headers:?}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_response_modifier_reaches_an_a2a_refusal_the_gateway_answers_itself() {
    // A refusal never reaches the proxy, which is where the modifier used to
    // live -- so this is exactly the response it could not touch before.
    let agent = agent("Alpha", &["echo"], true, true).await;
    let (url, shutdown) = start_with(
        "                allowMethods: [\"message/send\"]",
        &[agent.port],
        STAMP,
    )
    .await;

    let response = reqwest::Client::new()
        .post(&url)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tasks/cancel", "params": {}}))
        .send()
        .await
        .expect("the gateway should answer");
    let headers = headers_of(response).await;

    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "x-served-by" && v == "rusty"),
        "a refused method is still a response on this route: {headers:?}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_response_modifier_reaches_the_merged_agent_card() {
    // Discovery is answered by the gateway rather than forwarded, so this is
    // the other response the proxy never saw.
    let agent = agent("Alpha", &["echo"], true, true).await;
    let (url, shutdown) = start_with(
        &format!("                allowMethods: [\"message/send\"]\n{CARD_POLICY}"),
        &[agent.port],
        STAMP,
    )
    .await;

    let response = reqwest::Client::new()
        .get(format!("{url}/.well-known/agent-card.json"))
        .send()
        .await
        .expect("the gateway should answer");
    assert!(response.status().is_success(), "{}", response.status());
    let headers = headers_of(response).await;

    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "x-served-by" && v == "rusty"),
        "saw {headers:?}"
    );

    shutdown.cancel();
}

/// A `requestHeaderModifier` that touches all three operations.
const TAG: &str = "              requestHeaderModifier:\n                set:\n                  x-tenant: acme\n                add:\n                  x-scope: agents\n                remove: [x-drop-me]";

#[tokio::test]
async fn a_request_modifier_reaches_a_proxied_a2a_agent() {
    // An `a2a` route dispatches through the same `host` proxy that has always
    // applied this, but nothing asserted it, and "already works" is exactly
    // the claim worth testing rather than assuming.
    let agent = agent("Alpha", &["echo"], true, true).await;
    let (url, shutdown) = start_with(
        "                allowMethods: [\"message/send\"]",
        &[agent.port],
        TAG,
    )
    .await;

    let response = reqwest::Client::new()
        .post(&url)
        .header("x-drop-me", "please")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "message/send", "params": {}}))
        .send()
        .await
        .expect("the gateway should answer");
    assert!(response.status().is_success(), "{}", response.status());

    let seen = agent.seen_headers.lock().expect("lock");
    assert!(
        seen.iter().any(|(k, v)| k == "x-tenant" && v == "acme"),
        "saw {seen:?}"
    );
    assert!(
        seen.iter().any(|(k, v)| k == "x-scope" && v == "agents"),
        "saw {seen:?}"
    );
    assert!(
        !seen.iter().any(|(k, _)| k == "x-drop-me"),
        "`remove` must drop a header the caller sent: {seen:?}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_request_modifier_applies_to_a_call_the_a2a_policy_had_to_buffer() {
    // Gating reads the method out of the body, so an `a2a` route hands the
    // proxy an already-buffered request rather than a stream. The modifier has
    // to survive that second path too.
    let agent = agent("Alpha", &["echo"], true, true).await;
    let (url, shutdown) = start_with(
        "                denyMethods: [\"^tasks/cancel$\"]",
        &[agent.port],
        TAG,
    )
    .await;

    let response = reqwest::Client::new()
        .post(&url)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "message/send", "params": {}}))
        .send()
        .await
        .expect("the gateway should answer");
    assert!(response.status().is_success(), "{}", response.status());
    assert_eq!(agent.hits.load(Ordering::Relaxed), 1);

    let seen = agent.seen_headers.lock().expect("lock");
    assert!(
        seen.iter().any(|(k, v)| k == "x-tenant" && v == "acme"),
        "saw {seen:?}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_path_rewrite_reaches_a_proxied_a2a_agent() {
    // An `a2a` route dispatches through the same `host` proxy that has always
    // applied this. Asserted rather than assumed.
    let agent = agent("Alpha", &["echo"], true, true).await;
    let (url, shutdown) = start_at(
        "/a2a",
        "                allowMethods: [\"message/send\"]",
        &[agent.port],
        "              urlRewrite:\n                path:\n                  prefix: /rpc",
    )
    .await;

    let response = reqwest::Client::new()
        .post(format!("{url}/a2a/send"))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "message/send", "params": {}}))
        .send()
        .await
        .expect("the gateway should answer");
    assert!(response.status().is_success(), "{}", response.status());

    assert_eq!(
        *agent.seen_path.lock().expect("lock"),
        "/rpc/send",
        "the matched prefix is what a `prefix` rewrite replaces"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_path_rewrite_survives_the_buffering_the_a2a_policy_forces() {
    // Gating reads the method out of the body, so the proxy is handed an
    // already-buffered request rather than a stream. The rewrite is applied on
    // that second path too.
    let agent = agent("Alpha", &["echo"], true, true).await;
    let (url, shutdown) = start_at(
        "/a2a",
        "                denyMethods: [\"^tasks/cancel$\"]",
        &[agent.port],
        "              urlRewrite:\n                path:\n                  full: /fixed",
    )
    .await;

    let response = reqwest::Client::new()
        .post(format!("{url}/a2a/anything"))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "message/send", "params": {}}))
        .send()
        .await
        .expect("the gateway should answer");
    assert!(response.status().is_success(), "{}", response.status());
    assert_eq!(agent.hits.load(Ordering::Relaxed), 1);
    assert_eq!(*agent.seen_path.lock().expect("lock"), "/fixed");

    shutdown.cancel();
}

#[tokio::test]
async fn an_authority_rewrite_redirects_a2a_traffic_and_card_discovery_with_it() {
    // The backend address is a port with nothing on it; only the rewrite makes
    // this route work at all. Discovery has to follow, or the gateway serves a
    // card it fetched from an address it never sends traffic to -- and behind
    // an egress proxy that is the only route to the agents, no card at all.
    let real = agent("Alpha", &["echo"], true, true).await;
    let dead = free_port().await;
    let (url, shutdown) = start_with(
        &format!("                allowMethods: [\"message/send\"]\n{CARD_POLICY}"),
        &[dead],
        &format!(
            "              urlRewrite:\n                authority: \"127.0.0.1:{}\"",
            real.port
        ),
    )
    .await;

    // The merged card exists, which means the card fetch found the agent.
    let card: Value = reqwest::Client::new()
        .get(format!("{url}/.well-known/agent-card.json"))
        .send()
        .await
        .expect("the gateway should answer")
        .json()
        .await
        .expect("should be JSON");
    assert_eq!(
        card["skills"][0]["id"], "echo",
        "discovery must have reached the rewritten address: {card}"
    );

    // And so does the call itself.
    let response = call(&url, "message/send").await;
    assert_eq!(response["result"]["ok"], true);
    assert_eq!(real.hits.load(Ordering::Relaxed), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn a_path_rewrite_does_not_move_agent_card_discovery() {
    // The well-known path is the A2A spec's, not the route's. Asking an agent
    // for its card somewhere else finds nothing.
    let agent = agent("Alpha", &["echo"], true, true).await;
    let (url, shutdown) = start_with(
        &format!("                allowMethods: [\"message/send\"]\n{CARD_POLICY}"),
        &[agent.port],
        "              urlRewrite:\n                path:\n                  full: /fixed",
    )
    .await;

    let card: Value = reqwest::Client::new()
        .get(format!("{url}/.well-known/agent-card.json"))
        .send()
        .await
        .expect("the gateway should answer")
        .json()
        .await
        .expect("should be JSON");
    assert_eq!(
        card["skills"][0]["id"], "echo",
        "the card was still fetched from the well-known path: {card}"
    );

    shutdown.cancel();
}

/// An agent that answers `status` for its first `failures` JSON-RPC calls,
/// then succeeds. Its card is always served.
async fn flaky_agent(failures: usize, status: u16) -> (u16, Arc<AtomicUsize>) {
    use axum::{Router, extract::Request, routing::any};

    let port = free_port().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);

    let app = Router::new().fallback(any(move |request: Request| {
        let counter = Arc::clone(&counter);
        async move {
            let path = request.uri().path().to_string();
            if request.method() == axum::http::Method::GET
                && path.ends_with("/.well-known/agent-card.json")
            {
                return axum::response::Response::builder()
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        agent_card(
                            "Flaky",
                            &format!("http://127.0.0.1:{port}"),
                            &["echo"],
                            true,
                        )
                        .to_string(),
                    ))
                    .expect("response should build");
            }

            let bytes = axum::body::to_bytes(request.into_body(), 1 << 20)
                .await
                .unwrap_or_default();
            let seen = counter.fetch_add(1, Ordering::Relaxed);
            if seen < failures {
                return axum::response::Response::builder()
                    .status(status)
                    .body(axum::body::Body::from("upstream is unwell"))
                    .expect("response should build");
            }

            // Echo the method back so a replayed body can be checked.
            let method = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|call| call["method"].as_str().map(str::to_string))
                .unwrap_or_default();
            axum::response::Response::builder()
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": true, "saw": method}})
                        .to_string(),
                ))
                .expect("response should build")
        }
    }));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("agent should bind");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (port, calls)
}

const RETRY: &str =
    "              retry:\n                attempts: 2\n                codes: [503]";

#[tokio::test]
async fn a_listed_status_is_retried_on_an_a2a_route() {
    // An `a2a` route dispatches through the `host` proxy, which has always
    // retried. Asserted rather than assumed.
    let (port, calls) = flaky_agent(2, 503).await;
    let (url, shutdown) = start_with(
        "                allowMethods: [\"message/send\"]",
        &[port],
        RETRY,
    )
    .await;

    let response = call(&url, "message/send").await;
    assert_eq!(response["result"]["ok"], true, "{response}");
    assert_eq!(calls.load(Ordering::Relaxed), 3);

    shutdown.cancel();
}

#[tokio::test]
async fn a_retried_a2a_call_replays_the_body_the_policy_had_to_buffer() {
    // Gating reads the method out of the body, so the proxy is handed an
    // already-buffered request -- replayable by construction. An attempt after
    // the first must not arrive empty.
    let (port, calls) = flaky_agent(1, 503).await;
    let (url, shutdown) = start_with(
        "                denyMethods: [\"^tasks/cancel$\"]",
        &[port],
        RETRY,
    )
    .await;

    let response = call(&url, "message/send").await;
    assert_eq!(
        response["result"]["saw"], "message/send",
        "the replayed attempt carried the same body: {response}"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 2);

    shutdown.cancel();
}

#[tokio::test]
async fn a_refused_a2a_method_is_never_retried_because_it_is_never_sent() {
    // The refusal is the gateway's own response, produced before dispatch, so
    // there is no upstream attempt for a retry policy to repeat.
    let (port, calls) = flaky_agent(0, 503).await;
    let (url, shutdown) = start_with(
        "                denyMethods: [\"^tasks/cancel$\"]",
        &[port],
        RETRY,
    )
    .await;

    let response = call(&url, "tasks/cancel").await;
    assert_eq!(response["error"]["code"], -32011);
    assert_eq!(calls.load(Ordering::Relaxed), 0);

    shutdown.cancel();
}

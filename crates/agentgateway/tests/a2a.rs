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
    let hits = Arc::new(AtomicUsize::new(0));

    let recorder = Arc::clone(&calls);
    let counter = Arc::clone(&hits);

    let app = Router::new().fallback(any(move |request: Request| {
        let recorder = Arc::clone(&recorder);
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

    Agent { port, calls, hits }
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
    let port = free_port().await;
    let backends: String = agents
        .iter()
        .map(|p| format!("              - host: \"127.0.0.1:{p}\"\n"))
        .collect();

    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - name: agents
            policies:
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

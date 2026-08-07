//! The A2A policy: method gating and agent-card discovery.
//!
//! An `a2a` policy marks a route as carrying [Agent2Agent] traffic. The
//! gateway does not become an agent — it fronts them — so it takes only
//! [`rusty_a2a`]'s protocol types and leaves the agent harness alone. With
//! `default-features = false` that is a types-only dependency: no `axum`, no
//! `tonic`, no second `reqwest`.
//!
//! Two things happen on such a route:
//!
//! - **Method gating.** An A2A call names its operation in the JSON-RPC
//!   `method` field, so a route can permit `message/send` and refuse
//!   `tasks/cancel`. See [`gate`].
//! - **Agent card discovery.** The gateway serves a card for the agents behind
//!   the route at the well-known path, with the URL rewritten to its own. See
//!   [`card`].
//!
//! [Agent2Agent]: https://a2a-protocol.org

pub mod card;
pub mod gate;

use std::time::Duration;

use agentgateway_config::A2aPolicy;
use rusty_a2a::AGENT_CARD_WELL_KNOWN_PATH;
use rusty_a2a::types::jsonrpc::{JsonRpcRequest, JsonRpcResponse, RequestId};
use rusty_a2a::{A2aError, types::AgentCard};
use serde_json::Value;

pub use card::{Merged, Rejected};
pub use gate::{GateError, MethodGate};

/// Failure to build the A2A policy.
#[derive(Debug, thiserror::Error)]
pub enum A2aBuildError {
    /// A method pattern did not compile.
    #[error(transparent)]
    Gate(#[from] GateError),
}

/// How long the gateway waits for a backend agent's card at startup.
///
/// Short on purpose: an agent that is slow to answer should delay booting by
/// seconds, not minutes, and a card it fails to serve costs only its own
/// entry in the merged one.
const CARD_TIMEOUT: Duration = Duration::from_secs(5);

/// The A2A policy attached to a route.
pub struct A2aGateway {
    gate: MethodGate,
    /// Whether the route asked the gateway to serve a card at all.
    ///
    /// Distinct from `card` being `Some`: a route with no `agentCard` policy
    /// has no opinion about discovery, so the well-known path is an ordinary
    /// proxied request and must reach the agent. A route that *did* ask and
    /// whose agents served nothing usable is a different situation, and 503 is
    /// the honest answer there.
    serves_card: bool,
    /// The merged card, serialized once because it never changes.
    card: Option<Vec<u8>>,
}

impl std::fmt::Debug for A2aGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("A2aGateway")
            .field("serves_card", &self.serves_card)
            .field("card_assembled", &self.card.is_some())
            .finish_non_exhaustive()
    }
}

impl A2aGateway {
    /// Build the policy, fetching backend agent cards if one is to be served.
    ///
    /// `agents` are the `host:port` authorities of the backends on this route.
    /// An agent that cannot be reached, or serves a card that will not parse,
    /// is reported and skipped: one non-conformant agent must not break
    /// discovery for the rest.
    pub async fn build(
        policy: &A2aPolicy,
        agents: &[String],
        at: &str,
    ) -> Result<Self, A2aBuildError> {
        let gate = MethodGate::new(policy, at)?;

        let Some(card_policy) = policy.agent_card.as_ref() else {
            return Ok(A2aGateway {
                gate,
                serves_card: false,
                card: None,
            });
        };

        let client = reqwest::Client::builder()
            .timeout(CARD_TIMEOUT)
            .build()
            .unwrap_or_default();

        let mut cards: Vec<(String, AgentCard)> = Vec::new();
        for agent in agents {
            let url = format!("http://{agent}{AGENT_CARD_WELL_KNOWN_PATH}");
            match fetch(&client, &url).await {
                Ok(bytes) => match card::parse(agent, &bytes) {
                    Ok(card) => cards.push((agent.clone(), card)),
                    Err(rejected) => tracing::warn!(
                        route = %at,
                        agent = %rejected.source,
                        reason = %rejected.reason,
                        "agent card could not be parsed; excluding it from discovery"
                    ),
                },
                Err(err) => tracing::warn!(
                    route = %at,
                    %agent,
                    %err,
                    "agent card could not be fetched; excluding it from discovery"
                ),
            }
        }

        let merged = card::merge(cards, card_policy);
        if let Some(merged) = &merged {
            for collision in &merged.collisions {
                tracing::warn!(route = %at, "agent card skill collision: {collision}");
            }
            tracing::info!(
                route = %at,
                skills = merged.card.skills.len(),
                "serving a merged agent card"
            );
        } else {
            tracing::warn!(
                route = %at,
                "no agent card could be assembled; the well-known path will 503"
            );
        }

        Ok(A2aGateway {
            gate,
            serves_card: true,
            card: merged.map(|merged| {
                serde_json::to_vec(&merged.card).unwrap_or_else(|_| b"{}".to_vec())
            }),
        })
    }

    /// Whether the gateway should answer this request with its own card.
    ///
    /// False when the route configured no `agentCard`: discovery is then the
    /// agent's business and the request is proxied like any other.
    pub fn is_card_request(&self, method: &http::Method, path: &str) -> bool {
        self.serves_card
            && method == http::Method::GET
            && path.ends_with(AGENT_CARD_WELL_KNOWN_PATH)
    }

    /// The merged agent card, if one could be assembled.
    pub fn card(&self) -> Option<&[u8]> {
        self.card.as_deref()
    }

    /// Check an A2A request body against the method gate.
    ///
    /// A body that is not a JSON-RPC request is passed through rather than
    /// rejected: A2A also has REST and gRPC bindings, and refusing anything
    /// this gate cannot read would break them for no security benefit — the
    /// gate's job is to refuse *named* methods, not to be a schema validator.
    pub fn check(&self, body: &[u8]) -> Decision {
        let Ok(request) = serde_json::from_slice::<JsonRpcRequest>(body) else {
            return Decision::NotJsonRpc;
        };

        if self.gate.permits(&request.method) {
            Decision::Permitted {
                method: request.method,
            }
        } else {
            Decision::Refused {
                method: request.method.clone(),
                body: refusal(request.id),
            }
        }
    }
}

/// What the gate decided about a request.
#[derive(Debug)]
pub enum Decision {
    /// The method may be called.
    Permitted {
        /// The JSON-RPC method, for logs and traces.
        method: String,
    },
    /// The method is refused; answer with `body`.
    Refused {
        /// The method that was refused.
        method: String,
        /// A JSON-RPC error response.
        body: Vec<u8>,
    },
    /// Not a JSON-RPC request, so the gate has nothing to say.
    NotJsonRpc,
}

/// The JSON-RPC error body for a refused method.
///
/// Built from [`A2aError::PermissionDenied`] so the code is the one the spec
/// assigns (`-32011`) rather than a plausible-looking guess, and so the
/// envelope matches what any A2A client already parses.
fn refusal(id: RequestId) -> Vec<u8> {
    let error = A2aError::PermissionDenied("this method is not permitted on this route".into());
    serde_json::to_vec(&JsonRpcResponse::failure(id, &error))
        .unwrap_or_else(|_| b"{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32011}}".to_vec())
}

async fn fetch(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client.get(url).send().await.map_err(|err| err.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|err| err.to_string())
}

/// Extract a task id from an A2A request, for observability.
///
/// Best-effort: the field lives in different places depending on the method,
/// and a missing one is not an error.
pub fn task_id(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let params = value.get("params")?;
    params
        .get("taskId")
        .or_else(|| params.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gateway(deny: &[&str]) -> A2aGateway {
        A2aGateway {
            gate: MethodGate::new(
                &A2aPolicy {
                    deny_methods: deny.iter().map(|s| s.to_string()).collect(),
                    ..Default::default()
                },
                "test",
            )
            .expect("should compile"),
            serves_card: true,
            card: None,
        }
    }

    #[test]
    fn a_permitted_method_reports_its_name() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"message/send","params":{}}"#;
        match gateway(&[]).check(body) {
            Decision::Permitted { method } => assert_eq!(method, "message/send"),
            other => panic!("expected Permitted, got {other:?}"),
        }
    }

    #[test]
    fn a_refused_method_answers_with_the_spec_error_code() {
        let body = br#"{"jsonrpc":"2.0","id":7,"method":"tasks/cancel","params":{}}"#;
        match gateway(&["^tasks/cancel$"]).check(body) {
            Decision::Refused { method, body } => {
                assert_eq!(method, "tasks/cancel");
                let parsed: Value = serde_json::from_slice(&body).expect("should be JSON");
                assert_eq!(
                    parsed["error"]["code"], -32011,
                    "the spec assigns this code to PermissionDenied"
                );
                assert_eq!(parsed["id"], 7, "a client matches the response to its call");
                assert_eq!(parsed["jsonrpc"], "2.0");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn a_body_that_is_not_json_rpc_is_passed_through() {
        // A2A also has REST and gRPC bindings. Refusing what this gate cannot
        // read would break them for no security benefit.
        assert!(matches!(
            gateway(&["^tasks/"]).check(b"not json at all"),
            Decision::NotJsonRpc
        ));
        assert!(matches!(
            gateway(&["^tasks/"]).check(br#"{"message": {"role": "user"}}"#),
            Decision::NotJsonRpc
        ));
    }

    #[test]
    fn a_route_without_a_card_policy_does_not_claim_the_well_known_path() {
        // Discovery is then the agent's business, and the request is proxied
        // like any other.
        let mut gateway = gateway(&[]);
        gateway.serves_card = false;
        assert!(!gateway.is_card_request(&http::Method::GET, AGENT_CARD_WELL_KNOWN_PATH));
    }

    #[test]
    fn the_well_known_path_is_recognised_only_for_get() {
        let gateway = gateway(&[]);
        assert!(gateway.is_card_request(&http::Method::GET, AGENT_CARD_WELL_KNOWN_PATH));
        assert!(gateway.is_card_request(&http::Method::GET, "/agents/a/.well-known/agent-card.json"));
        assert!(
            !gateway.is_card_request(&http::Method::POST, AGENT_CARD_WELL_KNOWN_PATH),
            "a POST to that path is not a discovery request"
        );
        assert!(!gateway.is_card_request(&http::Method::GET, "/other"));
    }

    #[test]
    fn a_task_id_is_extracted_where_the_method_puts_it() {
        assert_eq!(
            task_id(br#"{"jsonrpc":"2.0","id":1,"method":"tasks/get","params":{"id":"task-1"}}"#),
            Some("task-1".to_string())
        );
        assert_eq!(
            task_id(br#"{"params":{"taskId":"task-2"}}"#),
            Some("task-2".to_string())
        );
        assert_eq!(task_id(br#"{"params":{}}"#), None, "a missing id is not an error");
        assert_eq!(task_id(b"garbage"), None);
    }
}

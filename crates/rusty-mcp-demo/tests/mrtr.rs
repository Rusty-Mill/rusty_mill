//! End-to-end MRTR tests against `DemoServer`.
//!
//! The unit tests cover sealing in isolation. What only shows up on the wire is
//! the round trip itself: the server returns an input request, the client
//! answers, and the retry lands on a handler that remembers nothing except what
//! it sealed.

use rmcp::{
    ClientHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, ClientInfo, ElicitRequestParams, ElicitResult,
        ElicitationAction, InputRequest, InputResponses, ProtocolVersion,
    },
    service::RunningService,
};

#[path = "../src/prompts.rs"]
mod prompts;
#[path = "../src/resources.rs"]
mod resources;
#[path = "../src/server.rs"]
mod server;
#[path = "../src/tools/mod.rs"]
mod tools;

use server::DemoServer;

#[derive(Clone)]
struct Client;

impl ClientHandler for Client {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        info
    }
}

async fn connect() -> RunningService<rmcp::RoleClient, Client> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);

    tokio::spawn(async move {
        let running = DemoServer::new()
            .serve(server_transport)
            .await
            .expect("server starts");
        let _ = running.waiting().await;
    });

    Client
        .serve(client_transport)
        .await
        .expect("client connects")
}

fn drop_table(table: &str) -> CallToolRequestParams {
    CallToolRequestParams::new("drop_table").with_arguments(
        serde_json::json!({ "table": table })
            .as_object()
            .cloned()
            .expect("object"),
    )
}

/// Build the client's answer to a confirmation.
fn answer(key: &str, action: ElicitationAction, confirm: Option<bool>) -> InputResponses {
    let mut result = serde_json::json!({ "action": action });
    if let Some(confirm) = confirm {
        result["content"] = serde_json::json!({ "confirm": confirm });
    }

    let mut responses = InputResponses::new();
    responses.insert(key.to_string(), result);
    responses
}

/// Make the first call and return the input request plus its request state.
async fn ask(client: &RunningService<rmcp::RoleClient, Client>, table: &str) -> (String, String) {
    // `call_tool_once` returns the intermediate result instead of driving the
    // rounds automatically, which is what lets the test act as the client.
    let response = client
        .call_tool_once(drop_table(table))
        .await
        .expect("call drop_table");

    let CallToolResponse::InputRequired(required) = response else {
        panic!("expected an input request, got {response:?}");
    };

    let requests = required.input_requests.expect("input requests");
    let (key, request) = requests.into_iter().next().expect("one request");

    // It must be an elicitation carrying the table name, or the user is being
    // asked to confirm something they cannot see.
    let InputRequest::Elicitation(elicit) = request else {
        panic!("expected an elicitation");
    };
    let ElicitRequestParams::FormElicitationParams { message, .. } = elicit.params else {
        panic!("expected a form elicitation");
    };
    assert!(message.contains(table), "prompt should name the table");

    (key, required.request_state.expect("request state"))
}

/// Retry the original call with answers and the echoed state.
async fn retry(
    client: &RunningService<rmcp::RoleClient, Client>,
    table: &str,
    state: &str,
    responses: InputResponses,
) -> CallToolResponse {
    let mut params = drop_table(table);
    params.request_state = Some(state.to_string());
    params.input_responses = Some(responses);

    client.call_tool_once(params).await.expect("retry")
}

fn text_of(response: CallToolResponse) -> String {
    let CallToolResponse::Complete(result) = response else {
        panic!("expected a completed result, got {response:?}");
    };
    result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content")
}

#[tokio::test]
async fn the_first_call_asks_instead_of_acting() {
    let client = connect().await;

    let (key, state) = ask(&client, "users").await;
    assert!(!key.is_empty());
    assert!(
        !state.is_empty(),
        "the server must hand back a request state"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn confirming_completes_the_operation() {
    let client = connect().await;
    let (key, state) = ask(&client, "users").await;

    let response = retry(
        &client,
        "users",
        &state,
        answer(&key, ElicitationAction::Accept, Some(true)),
    )
    .await;

    assert_eq!(text_of(response), "dropped `users`");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn declining_leaves_it_alone() {
    let client = connect().await;
    let (key, state) = ask(&client, "users").await;

    let response = retry(
        &client,
        "users",
        &state,
        answer(&key, ElicitationAction::Decline, None),
    )
    .await;

    assert_eq!(text_of(response), "left `users` alone");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn accepting_with_confirm_false_is_not_consent() {
    // The user clicked through the form but said no.
    let client = connect().await;
    let (key, state) = ask(&client, "users").await;

    let response = retry(
        &client,
        "users",
        &state,
        answer(&key, ElicitationAction::Accept, Some(false)),
    )
    .await;

    assert_eq!(text_of(response), "left `users` alone");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn a_tampered_request_state_is_rejected() {
    let client = connect().await;
    let (key, state) = ask(&client, "users").await;

    // Flip a character in the sealed value.
    let mut tampered = state.clone();
    let last = tampered.pop().expect("non-empty");
    tampered.push(if last == 'A' { 'B' } else { 'A' });

    let mut params = drop_table("users");
    params.request_state = Some(tampered);
    params.input_responses = Some(answer(&key, ElicitationAction::Accept, Some(true)));

    assert!(
        client.call_tool_once(params).await.is_err(),
        "a forged state must not be honoured"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn the_confirmed_table_comes_from_the_sealed_state() {
    // The confused-operation case: confirm one table, then retry naming a
    // different one. The server must act on what the user actually saw.
    let client = connect().await;
    let (key, state) = ask(&client, "users").await;

    let response = retry(
        &client,
        "orders", // <- changed by the client between rounds
        &state,
        answer(&key, ElicitationAction::Accept, Some(true)),
    )
    .await;

    assert_eq!(
        text_of(response),
        "dropped `users`",
        "the sealed table must win over the retry arguments"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn answers_without_a_request_state_are_rejected() {
    let client = connect().await;

    let mut params = drop_table("users");
    params.input_responses = Some(answer(
        "confirm-drop",
        ElicitationAction::Accept,
        Some(true),
    ));
    // No request_state.

    assert!(
        client.call_tool_once(params).await.is_err(),
        "answers with no state must not be silently restarted"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn an_elicit_result_deserializes_from_the_wire_form() {
    // Guards the shape the demo depends on: whatever the client sends must
    // parse back into an ElicitResult for `accepted()` to read.
    let raw = serde_json::json!({ "action": "accept", "content": { "confirm": true } });
    let parsed: ElicitResult = serde_json::from_value(raw).expect("parses");
    assert_eq!(parsed.action, ElicitationAction::Accept);
}

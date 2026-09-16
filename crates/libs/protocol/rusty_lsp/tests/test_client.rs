//! Self-test for the exported [`rusty_lsp::testing::TestClient`] harness:
//! drives a small backend through the harness's own API surface.

use rusty_lsp::error::Result;
use rusty_lsp::lsp::{
    Hover, HoverParams, InitializeParams, InitializeResult, MessageType, ServerCapabilities,
};
use rusty_lsp::testing::TestClient;
use rusty_lsp::{Client, LanguageServer};
use serde_json::{Value, json};

struct Backend {
    client: Client,
}

impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                hover_provider: Some(true.into()),
                ..Default::default()
            },
            server_info: None,
        })
    }

    async fn initialized(&self) {
        let _ = self.client.log_message(MessageType::Info, "ready");
    }

    async fn hover(&self, _params: HoverParams) -> Result<Option<Hover>> {
        Ok(Some(Hover::markdown("hi from hover")))
    }

    async fn handle_request(&self, method: &str, _params: Option<Value>) -> Result<Value> {
        match method {
            // Round-trips a server->client request so the test can exercise
            // start_request / recv_request / respond / response.
            "test/ask_config" => {
                let values = self
                    .client
                    .configuration(vec![rusty_lsp::lsp::ConfigurationItem {
                        section: Some("editor.tabSize".to_owned()),
                        scope_uri: None,
                    }])
                    .await?;
                Ok(json!(values))
            }
            other => Err(rusty_lsp::Error::method_not_found(other.to_owned())),
        }
    }
}

#[tokio::test]
async fn test_client_drives_the_full_lifecycle() {
    let mut client = TestClient::spawn(|client| Backend { client });

    let init = client
        .initialize(InitializeParams::default())
        .await
        .expect("initialize");
    assert_eq!(init.capabilities.hover_provider, Some(true.into()));

    // The `initialized` hook fired and its notification is retrievable even
    // after other traffic.
    let hover: Option<Hover> = client
        .request(
            "textDocument/hover",
            json!({
                "textDocument": {"uri": "file:///a.txt"},
                "position": {"line": 0, "character": 0},
            }),
        )
        .await
        .expect("hover");
    assert!(hover.is_some());
    let note = client
        .recv_notification("window/logMessage")
        .await
        .expect("buffered notification");
    assert_eq!(note.params.expect("params")["message"], json!("ready"));

    client.shutdown_and_exit().await.expect("clean teardown");
}

#[tokio::test]
async fn test_client_answers_server_to_client_requests() {
    let mut client = TestClient::spawn(|client| Backend { client });
    client
        .initialize(InitializeParams::default())
        .await
        .expect("initialize");

    let id = client
        .start_request("test/ask_config", json!({}))
        .await
        .expect("send");
    let config_req = client
        .recv_request("workspace/configuration")
        .await
        .expect("server asks for config");
    client
        .respond(config_req.id, json!([4]))
        .await
        .expect("answer config");

    let response = client.response(&id).await.expect("handler result");
    assert_eq!(response.result.expect("result"), json!([4]));

    client.shutdown_and_exit().await.expect("clean teardown");
}

#[tokio::test]
async fn test_client_error_responses_become_errors() {
    let mut client = TestClient::spawn(|client| Backend { client });
    client
        .initialize(InitializeParams::default())
        .await
        .expect("initialize");

    let err = client
        .request::<_, Value>("no/suchMethod", json!({}))
        .await
        .expect_err("unknown method");
    match err {
        rusty_lsp::Error::Response(e) => {
            assert_eq!(e.code, rusty_lsp::error::codes::METHOD_NOT_FOUND)
        }
        other => panic!("unexpected error: {other}"),
    }

    client.shutdown_and_exit().await.expect("clean teardown");
}

#[tokio::test]
async fn test_client_receives_fail_instead_of_hanging() {
    let mut client = TestClient::spawn(|client| Backend { client })
        .with_timeout(std::time::Duration::from_millis(200));
    client
        .initialize(InitializeParams::default())
        .await
        .expect("initialize");

    // Nothing will ever send this notification; the receive must fail with
    // a descriptive error rather than hang the test.
    let err = client
        .recv_notification("never/sent")
        .await
        .expect_err("times out");
    let text = err.to_string();
    assert!(text.contains("no message"), "was: {text}");
}

#[tokio::test]
async fn spawn_configured_applies_server_options() {
    // A queue limit of 0 rejects Client-originated sends; the backend's
    // `initialized` log therefore never arrives, but responses still do.
    let mut client = TestClient::spawn_configured(
        |server| server.with_max_concurrent_requests(1),
        |client| Backend { client },
    );
    let init = client
        .initialize(InitializeParams::default())
        .await
        .expect("initialize under a concurrency cap");
    assert_eq!(init.capabilities.hover_provider, Some(true.into()));
    client.shutdown_and_exit().await.expect("clean teardown");
}

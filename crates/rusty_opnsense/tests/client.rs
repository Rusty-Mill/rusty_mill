mod support;

use rusty_opnsense::{Error, OpnsenseClient, OpnsenseConfig, ServiceAction};
use support::MockResponse;

fn client(base_url: String) -> OpnsenseClient {
    OpnsenseClient::new(OpnsenseConfig {
        base_url,
        key: "test-key".to_string(),
        secret: "test-secret".to_string(),
        insecure: false,
        timeout: None,
    })
}

#[tokio::test]
async fn system_status_returns_the_body_as_is() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"product_version":"25.1","status":"ok"}"#,
    )]);

    let status = client(base_url).system_status().await.expect("status");

    assert_eq!(status["status"], "ok");
}

#[tokio::test]
async fn list_services_passes_through_the_search_envelope() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"rows":[{"id":"unbound","name":"Unbound DNS","running":1}],"rowCount":1}"#,
    )]);

    let services = client(base_url)
        .list_services()
        .await
        .expect("list_services");

    assert_eq!(services["rows"][0]["id"], "unbound");
}

#[tokio::test]
async fn service_control_posts_to_the_action_and_name_path() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"{"response":"OK"}"#)]);

    let result = client(base_url)
        .service_control("unbound", ServiceAction::Restart)
        .await
        .expect("service_control");

    assert_eq!(result["response"], "OK");
}

#[tokio::test]
async fn a_4xx_status_becomes_an_api_error() {
    let base_url = support::spawn(vec![MockResponse::status(
        403,
        "Forbidden",
        r#"{"message":"invalid api key"}"#,
    )]);

    let err = client(base_url)
        .system_status()
        .await
        .expect_err("a forbidden response should fail");

    match err {
        Error::Api { status, body } => {
            assert_eq!(status, 403);
            assert!(body.contains("invalid api key"), "body: {body}");
        }
        other => panic!("expected Error::Api, got {other:?}"),
    }
}

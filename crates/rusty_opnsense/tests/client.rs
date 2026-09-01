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
async fn list_firewall_rules_passes_through_the_search_envelope() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"rows":[{"uuid":"abc-123","enabled":"1","action":"pass"}],"rowCount":1}"#,
    )]);

    let rules = client(base_url)
        .list_firewall_rules()
        .await
        .expect("list_firewall_rules");

    assert_eq!(rules["rows"][0]["uuid"], "abc-123");
}

#[tokio::test]
async fn get_firewall_rule_builds_the_uuid_path() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"rule":{"action":"pass","interface":"lan"}}"#,
    )]);

    let rule = client(base_url)
        .get_firewall_rule("abc-123")
        .await
        .expect("get_firewall_rule");

    assert_eq!(rule["rule"]["interface"], "lan");
}

#[tokio::test]
async fn create_firewall_rule_wraps_the_fields_under_rule() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"result":"saved","uuid":"new-uuid"}"#,
    )]);

    let result = client(base_url)
        .create_firewall_rule(serde_json::json!({
            "action": "pass",
            "interface": "lan",
            "description": "allow ssh",
        }))
        .await
        .expect("create_firewall_rule");

    assert_eq!(result["result"], "saved");
}

#[tokio::test]
async fn update_firewall_rule_builds_the_uuid_path() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"{"result":"saved"}"#)]);

    let result = client(base_url)
        .update_firewall_rule("abc-123", serde_json::json!({ "enabled": "0" }))
        .await
        .expect("update_firewall_rule");

    assert_eq!(result["result"], "saved");
}

#[tokio::test]
async fn delete_firewall_rule_builds_the_uuid_path() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"{"result":"deleted"}"#)]);

    let result = client(base_url)
        .delete_firewall_rule("abc-123")
        .await
        .expect("delete_firewall_rule");

    assert_eq!(result["result"], "deleted");
}

#[tokio::test]
async fn toggle_firewall_rule_with_no_explicit_state_flips_it() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"{"changed":true}"#)]);

    let result = client(base_url)
        .toggle_firewall_rule("abc-123", None)
        .await
        .expect("toggle_firewall_rule");

    assert_eq!(result["changed"], true);
}

#[tokio::test]
async fn toggle_firewall_rule_with_an_explicit_state_sets_it() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"{"changed":true}"#)]);

    let result = client(base_url)
        .toggle_firewall_rule("abc-123", Some(false))
        .await
        .expect("toggle_firewall_rule");

    assert_eq!(result["changed"], true);
}

#[tokio::test]
async fn apply_firewall_changes_posts_to_the_apply_endpoint() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"{"status":"ok"}"#)]);

    let result = client(base_url)
        .apply_firewall_changes()
        .await
        .expect("apply_firewall_changes");

    assert_eq!(result["status"], "ok");
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

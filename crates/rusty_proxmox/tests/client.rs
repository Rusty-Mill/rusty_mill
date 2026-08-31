mod support;

use rusty_proxmox::{Error, GuestKind, PowerAction, ProxmoxClient, ProxmoxConfig};
use support::MockResponse;

fn client(base_url: String) -> ProxmoxClient {
    ProxmoxClient::new(ProxmoxConfig {
        base_url,
        token_id: "automation@pve!test".to_string(),
        token_secret: "secret".to_string(),
        insecure: false,
        timeout: None,
    })
}

#[tokio::test]
async fn list_nodes_unwraps_the_data_field() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":[{"node":"pve","status":"online"}]}"#,
    )]);

    let nodes = client(base_url).list_nodes().await.expect("list_nodes");

    assert_eq!(nodes[0]["node"], "pve");
    assert_eq!(nodes[0]["status"], "online");
}

#[tokio::test]
async fn list_guests_builds_the_kind_specific_path() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":[{"vmid":100,"name":"web","status":"running"}]}"#,
    )]);

    let guests = client(base_url)
        .list_guests("pve", GuestKind::Qemu)
        .await
        .expect("list_guests");

    assert_eq!(guests[0]["vmid"], 100);
}

#[tokio::test]
async fn guest_power_returns_the_task_upid() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":"UPID:pve:00001234:0000ABCD:00000000:qmstart:100:automation@pve!test:"}"#,
    )]);

    let upid = client(base_url)
        .guest_power("pve", GuestKind::Qemu, 100, PowerAction::Start)
        .await
        .expect("guest_power");

    assert!(upid.starts_with("UPID:pve:"), "unexpected upid: {upid}");
}

#[tokio::test]
async fn a_4xx_status_becomes_an_api_error() {
    let base_url = support::spawn(vec![MockResponse::status(
        401,
        "Unauthorized",
        r#"{"data":null,"errors":{"token":"invalid API token"}}"#,
    )]);

    let err = client(base_url)
        .list_nodes()
        .await
        .expect_err("an unauthorized response should fail");

    match err {
        Error::Api { status, body } => {
            assert_eq!(status, 401);
            assert!(body.contains("invalid API token"), "body: {body}");
        }
        other => panic!("expected Error::Api, got {other:?}"),
    }
}

#[tokio::test]
async fn a_response_without_a_data_field_is_a_shape_error() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"{"unexpected":true}"#)]);

    let err = client(base_url)
        .list_nodes()
        .await
        .expect_err("a response with no `data` field should fail");

    assert!(matches!(err, Error::MissingData(_)), "got {err:?}");
}

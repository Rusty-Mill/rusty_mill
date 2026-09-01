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
async fn task_status_reports_a_finished_task() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":{"status":"stopped","exitstatus":"OK"}}"#,
    )]);

    let status = client(base_url)
        .task_status(
            "pve",
            "UPID:pve:00001234:0000ABCD:00000000:qmstart:100:automation@pve!test:",
        )
        .await
        .expect("task_status");

    assert_eq!(status["status"], "stopped");
    assert_eq!(status["exitstatus"], "OK");
}

#[tokio::test]
async fn task_log_returns_the_log_lines() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":[{"n":1,"t":"TASK OK"}]}"#,
    )]);

    let log = client(base_url)
        .task_log(
            "pve",
            "UPID:pve:00001234:0000ABCD:00000000:qmstart:100:automation@pve!test:",
        )
        .await
        .expect("task_log");

    assert_eq!(log[0]["t"], "TASK OK");
}

#[tokio::test]
async fn guest_config_returns_the_data_field() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":{"cores":2,"memory":2048}}"#,
    )]);

    let config = client(base_url)
        .guest_config("pve", GuestKind::Qemu, 100)
        .await
        .expect("guest_config");

    assert_eq!(config["cores"], 2);
}

#[tokio::test]
async fn update_guest_config_sends_the_fields_as_a_json_body() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"{"data":null}"#)]);

    let result = client(base_url)
        .update_guest_config(
            "pve",
            GuestKind::Qemu,
            100,
            serde_json::json!({ "memory": 4096 }),
        )
        .await
        .expect("update_guest_config");

    assert!(result.is_null());
}

#[tokio::test]
async fn create_guest_returns_the_task_upid() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":"UPID:pve:00001234:0000ABCD:00000000:qmcreate:200:automation@pve!test:"}"#,
    )]);

    let upid = client(base_url)
        .create_guest(
            "pve",
            GuestKind::Qemu,
            serde_json::json!({ "vmid": 200, "ostype": "l26" }),
        )
        .await
        .expect("create_guest");

    assert!(upid.starts_with("UPID:pve:"), "unexpected upid: {upid}");
}

#[tokio::test]
async fn delete_guest_returns_the_task_upid() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":"UPID:pve:00001234:0000ABCD:00000000:qmdestroy:100:automation@pve!test:"}"#,
    )]);

    let upid = client(base_url)
        .delete_guest("pve", GuestKind::Qemu, 100)
        .await
        .expect("delete_guest");

    assert!(upid.starts_with("UPID:pve:"), "unexpected upid: {upid}");
}

#[tokio::test]
async fn clone_guest_returns_the_task_upid() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":"UPID:pve:00001234:0000ABCD:00000000:qmclone:100:automation@pve!test:"}"#,
    )]);

    let upid = client(base_url)
        .clone_guest(
            "pve",
            GuestKind::Qemu,
            100,
            serde_json::json!({ "newid": 200 }),
        )
        .await
        .expect("clone_guest");

    assert!(upid.starts_with("UPID:pve:"), "unexpected upid: {upid}");
}

#[tokio::test]
async fn list_snapshots_unwraps_the_data_field() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":[{"name":"before-upgrade","snaptime":1735689600}]}"#,
    )]);

    let snapshots = client(base_url)
        .list_snapshots("pve", GuestKind::Qemu, 100)
        .await
        .expect("list_snapshots");

    assert_eq!(snapshots[0]["name"], "before-upgrade");
}

#[tokio::test]
async fn create_snapshot_returns_the_task_upid() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":"UPID:pve:00001234:0000ABCD:00000000:qmsnapshot:100:automation@pve!test:"}"#,
    )]);

    let upid = client(base_url)
        .create_snapshot(
            "pve",
            GuestKind::Qemu,
            100,
            serde_json::json!({ "snapname": "before-upgrade" }),
        )
        .await
        .expect("create_snapshot");

    assert!(upid.starts_with("UPID:pve:"), "unexpected upid: {upid}");
}

#[tokio::test]
async fn delete_snapshot_returns_the_task_upid() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":"UPID:pve:00001234:0000ABCD:00000000:qmdelsnapshot:100:automation@pve!test:"}"#,
    )]);

    let upid = client(base_url)
        .delete_snapshot("pve", GuestKind::Qemu, 100, "before-upgrade")
        .await
        .expect("delete_snapshot");

    assert!(upid.starts_with("UPID:pve:"), "unexpected upid: {upid}");
}

#[tokio::test]
async fn rollback_snapshot_returns_the_task_upid() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":"UPID:pve:00001234:0000ABCD:00000000:qmrollback:100:automation@pve!test:"}"#,
    )]);

    let upid = client(base_url)
        .rollback_snapshot("pve", GuestKind::Qemu, 100, "before-upgrade")
        .await
        .expect("rollback_snapshot");

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

//! End-to-end tests: a real client talking to [`HomelabServer`] over an
//! in-memory duplex pipe, exercising the same code path as the stdio
//! transport.

mod support;

// The binary's modules, compiled into the test -- `crate::` paths inside
// them (e.g. `crate::server::HomelabServer`) resolve against this test
// binary's own module tree, which mirrors `src/`'s exactly.
#[path = "../src/json_result.rs"]
mod json_result;
#[path = "../src/server.rs"]
mod server;
#[path = "../src/tools/mod.rs"]
mod tools;

use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientInfo, ProtocolVersion},
    service::RunningService,
};
use rusty_fedora::{FedoraAgentClient, FedoraAgentConfig};
use rusty_opnsense::{OpnsenseClient, OpnsenseConfig};
use rusty_proxmox::{ProxmoxClient, ProxmoxConfig};
use server::HomelabServer;
use support::MockResponse;

/// A client that asks for 2026-07-28.
#[derive(Clone)]
struct Client;

impl ClientHandler for Client {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        info
    }
}

async fn connect(server: HomelabServer) -> RunningService<rmcp::RoleClient, Client> {
    let (server_transport, client_transport) = tokio::io::duplex(4096);

    tokio::spawn(async move {
        let running = server
            .serve(server_transport)
            .await
            .expect("server should start");
        let _ = running.waiting().await;
    });

    Client
        .serve(client_transport)
        .await
        .expect("client should connect")
}

fn call(name: &'static str, args: serde_json::Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name).with_arguments(
        args.as_object()
            .cloned()
            .expect("tool arguments must be a JSON object"),
    )
}

fn structured(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    result
        .structured_content
        .clone()
        .expect("tool should return structured content")
}

fn proxmox_client(base_url: String) -> ProxmoxClient {
    ProxmoxClient::new(ProxmoxConfig {
        base_url,
        token_id: "automation@pve!test".to_string(),
        token_secret: "secret".to_string(),
        insecure: false,
        timeout: None,
    })
}

fn opnsense_client(base_url: String) -> OpnsenseClient {
    OpnsenseClient::new(OpnsenseConfig {
        base_url,
        key: "key".to_string(),
        secret: "secret".to_string(),
        insecure: false,
        timeout: None,
    })
}

fn fedora_client(base_url: String) -> FedoraAgentClient {
    FedoraAgentClient::new(FedoraAgentConfig {
        base_url,
        timeout: None,
    })
}

#[tokio::test]
async fn every_backend_contributes_its_tools() {
    let client = connect(HomelabServer::new(None, None, None)).await;

    let tools = client.list_tools(None).await.expect("tools/list");
    let mut names: Vec<&str> = tools.tools.iter().map(|t| t.name.as_ref()).collect();
    names.sort_unstable();

    assert_eq!(
        names,
        [
            "fedora_dnf_install",
            "fedora_dnf_list_updates",
            "fedora_dnf_remove",
            "fedora_list_services",
            "fedora_read_config",
            "fedora_read_journal",
            "fedora_service_control",
            "fedora_system_status",
            "fedora_task_status",
            "fedora_write_config",
            "opnsense_apply_firewall_changes",
            "opnsense_apply_vlan_changes",
            "opnsense_create_firewall_rule",
            "opnsense_create_vlan",
            "opnsense_delete_firewall_rule",
            "opnsense_delete_vlan",
            "opnsense_download_backup",
            "opnsense_get_firewall_rule",
            "opnsense_get_vlan",
            "opnsense_list_arp_entries",
            "opnsense_list_backup_providers",
            "opnsense_list_backups",
            "opnsense_list_dhcp_leases",
            "opnsense_list_firewall_aliases",
            "opnsense_list_firewall_rules",
            "opnsense_list_gateways",
            "opnsense_list_interfaces",
            "opnsense_list_routes",
            "opnsense_list_services",
            "opnsense_list_vlans",
            "opnsense_restore_backup",
            "opnsense_service_control",
            "opnsense_system_status",
            "opnsense_toggle_firewall_rule",
            "opnsense_update_firewall_rule",
            "opnsense_update_vlan",
            "proxmox_clone_guest",
            "proxmox_cluster_resources",
            "proxmox_create_guest",
            "proxmox_create_snapshot",
            "proxmox_delete_guest",
            "proxmox_delete_snapshot",
            "proxmox_guest_config",
            "proxmox_guest_power",
            "proxmox_guest_status",
            "proxmox_list_backup_jobs",
            "proxmox_list_guests",
            "proxmox_list_nodes",
            "proxmox_list_snapshots",
            "proxmox_list_storage",
            "proxmox_migrate_guest",
            "proxmox_node_status",
            "proxmox_node_storage_status",
            "proxmox_rollback_snapshot",
            "proxmox_run_backup",
            "proxmox_task_log",
            "proxmox_task_status",
            "proxmox_update_guest_config",
        ]
    );

    for tool in &tools.tools {
        assert!(
            tool.description.as_ref().is_some_and(|d| !d.is_empty()),
            "tool {} is missing a description",
            tool.name
        );
    }

    client.cancel().await.expect("cancel");
}

/// MCP requires a tool's structured output -- and the `outputSchema` a
/// client validates it against -- to be a JSON object at the top level.
/// `serde_json::Value` on its own doesn't guarantee that (some backend
/// endpoints return a bare array), which is exactly what broke every
/// passthrough tool here once: a strict client (Claude Code/Cowork) rejected
/// the whole `tools/list` result and refused to start the server at all.
/// `JsonResult` (see `src/json_result.rs`) is the fix -- this guards it
/// stays fixed.
#[tokio::test]
async fn every_declared_output_schema_is_a_json_object() {
    let client = connect(HomelabServer::new(None, None, None)).await;

    let tools = client.list_tools(None).await.expect("tools/list");
    for tool in &tools.tools {
        if let Some(schema) = &tool.output_schema {
            assert_eq!(
                schema.get("type").and_then(|t| t.as_str()),
                Some("object"),
                "tool {} has a non-object outputSchema: {schema:?}",
                tool.name
            );
        }
    }

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn an_unconfigured_backend_fails_with_a_clear_error() {
    let client = connect(HomelabServer::new(None, None, None)).await;

    let err = client
        .call_tool(call("proxmox_list_nodes", serde_json::json!({})))
        .await
        .expect_err("proxmox_list_nodes should fail when Proxmox isn't configured");

    assert!(
        err.to_string().contains("Proxmox is not configured"),
        "unexpected error: {err}"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn proxmox_list_nodes_returns_structured_data_from_the_real_client_path() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":[{"node":"pve","status":"online"}]}"#,
    )]);
    let server = HomelabServer::new(Some(proxmox_client(base_url)), None, None);
    let client = connect(server).await;

    let result = client
        .call_tool(call("proxmox_list_nodes", serde_json::json!({})))
        .await
        .expect("call proxmox_list_nodes");

    assert_ne!(result.is_error, Some(true));
    assert_eq!(structured(&result)["result"][0]["node"], "pve");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn a_proxmox_api_error_surfaces_as_a_protocol_error() {
    let base_url = support::spawn(vec![MockResponse::status(
        401,
        "Unauthorized",
        r#"{"data":null,"errors":{"token":"invalid API token"}}"#,
    )]);
    let server = HomelabServer::new(Some(proxmox_client(base_url)), None, None);
    let client = connect(server).await;

    let err = client
        .call_tool(call("proxmox_list_nodes", serde_json::json!({})))
        .await
        .expect_err("an unauthorized upstream response should fail the tool call");

    assert!(
        err.to_string().contains("invalid API token"),
        "unexpected error: {err}"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn proxmox_task_status_returns_structured_data_from_the_real_client_path() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":{"status":"stopped","exitstatus":"OK"}}"#,
    )]);
    let server = HomelabServer::new(Some(proxmox_client(base_url)), None, None);
    let client = connect(server).await;

    let result = client
        .call_tool(call(
            "proxmox_task_status",
            serde_json::json!({
                "node": "pve",
                "upid": "UPID:pve:00001234:0000ABCD:00000000:qmstart:100:automation@pve!test:",
            }),
        ))
        .await
        .expect("call proxmox_task_status");

    assert_ne!(result.is_error, Some(true));
    assert_eq!(structured(&result)["result"]["exitstatus"], "OK");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn opnsense_system_status_returns_structured_data_from_the_real_client_path() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"{"status":"ok"}"#)]);
    let server = HomelabServer::new(None, Some(opnsense_client(base_url)), None);
    let client = connect(server).await;

    let result = client
        .call_tool(call("opnsense_system_status", serde_json::json!({})))
        .await
        .expect("call opnsense_system_status");

    assert_ne!(result.is_error, Some(true));
    assert_eq!(structured(&result)["result"]["status"], "ok");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn opnsense_create_firewall_rule_sends_the_rule_as_a_json_body() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"result":"saved","uuid":"new-uuid"}"#,
    )]);
    let server = HomelabServer::new(None, Some(opnsense_client(base_url)), None);
    let client = connect(server).await;

    let result = client
        .call_tool(call(
            "opnsense_create_firewall_rule",
            serde_json::json!({
                "rule": {
                    "action": "pass",
                    "interface": "lan",
                    "direction": "in",
                    "protocol": "TCP",
                    "destination_port": "22",
                    "description": "allow ssh",
                },
            }),
        ))
        .await
        .expect("call opnsense_create_firewall_rule");

    assert_ne!(result.is_error, Some(true));
    assert_eq!(structured(&result)["result"]["uuid"], "new-uuid");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn opnsense_create_vlan_sends_the_vlan_as_a_json_body() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"result":"saved","uuid":"new-vlan-uuid"}"#,
    )]);
    let server = HomelabServer::new(None, Some(opnsense_client(base_url)), None);
    let client = connect(server).await;

    let result = client
        .call_tool(call(
            "opnsense_create_vlan",
            serde_json::json!({
                "vlan": { "if": "igc0", "tag": "20", "descr": "guest network" },
            }),
        ))
        .await
        .expect("call opnsense_create_vlan");

    assert_ne!(result.is_error, Some(true));
    assert_eq!(structured(&result)["result"]["uuid"], "new-vlan-uuid");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn opnsense_download_backup_returns_the_raw_xml_as_plain_text() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"<?xml version="1.0"?><opnsense><version>25.7</version></opnsense>"#,
    )]);
    let server = HomelabServer::new(None, Some(opnsense_client(base_url)), None);
    let client = connect(server).await;

    let result = client
        .call_tool(call(
            "opnsense_download_backup",
            serde_json::json!({ "host": "this" }),
        ))
        .await
        .expect("call opnsense_download_backup");

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("download_backup returns text");
    assert!(text.contains("<opnsense>"), "unexpected body: {text}");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn opnsense_list_dhcp_leases_returns_structured_data_from_the_real_client_path() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"rows":[{"address":"10.0.0.42","hostname":"nas"}],"rowCount":1}"#,
    )]);
    let server = HomelabServer::new(None, Some(opnsense_client(base_url)), None);
    let client = connect(server).await;

    let result = client
        .call_tool(call("opnsense_list_dhcp_leases", serde_json::json!({})))
        .await
        .expect("call opnsense_list_dhcp_leases");

    assert_ne!(result.is_error, Some(true));
    assert_eq!(structured(&result)["result"]["rows"][0]["hostname"], "nas");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn proxmox_update_guest_config_sends_the_config_as_a_json_body() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"{"data":null}"#)]);
    let server = HomelabServer::new(Some(proxmox_client(base_url)), None, None);
    let client = connect(server).await;

    let result = client
        .call_tool(call(
            "proxmox_update_guest_config",
            serde_json::json!({
                "node": "pve",
                "kind": "qemu",
                "vmid": 100,
                "config": { "memory": 4096 },
            }),
        ))
        .await
        .expect("call proxmox_update_guest_config");

    assert_ne!(result.is_error, Some(true));
    assert!(structured(&result)["result"].is_null());

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn proxmox_create_guest_returns_the_task_upid_as_plain_text() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":"UPID:pve:00001234:0000ABCD:00000000:qmcreate:200:automation@pve!test:"}"#,
    )]);
    let server = HomelabServer::new(Some(proxmox_client(base_url)), None, None);
    let client = connect(server).await;

    let result = client
        .call_tool(call(
            "proxmox_create_guest",
            serde_json::json!({
                "node": "pve",
                "kind": "qemu",
                "config": { "vmid": 200, "ostype": "l26" },
            }),
        ))
        .await
        .expect("call proxmox_create_guest");

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("create_guest returns text");
    assert!(text.starts_with("UPID:pve:"), "unexpected upid: {text}");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn proxmox_cluster_resources_with_a_filter_returns_structured_data() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":[{"type":"storage","storage":"local"}]}"#,
    )]);
    let server = HomelabServer::new(Some(proxmox_client(base_url)), None, None);
    let client = connect(server).await;

    let result = client
        .call_tool(call(
            "proxmox_cluster_resources",
            serde_json::json!({ "resource_type": "storage" }),
        ))
        .await
        .expect("call proxmox_cluster_resources");

    assert_ne!(result.is_error, Some(true));
    assert_eq!(structured(&result)["result"][0]["storage"], "local");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn proxmox_guest_power_returns_the_task_upid_as_plain_text() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"data":"UPID:pve:00001234:0000ABCD:00000000:qmstart:100:automation@pve!test:"}"#,
    )]);
    let server = HomelabServer::new(Some(proxmox_client(base_url)), None, None);
    let client = connect(server).await;

    let result = client
        .call_tool(call(
            "proxmox_guest_power",
            serde_json::json!({ "node": "pve", "kind": "qemu", "vmid": 100, "action": "start" }),
        ))
        .await
        .expect("call proxmox_guest_power");

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("guest_power returns text");
    assert!(text.starts_with("UPID:pve:"), "unexpected upid: {text}");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn fedora_system_status_returns_structured_data_from_the_real_client_path() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"hostname":"baileyai","uptime_seconds":12345}"#,
    )]);
    let server = HomelabServer::new(None, None, Some(fedora_client(base_url)));
    let client = connect(server).await;

    let result = client
        .call_tool(call("fedora_system_status", serde_json::json!({})))
        .await
        .expect("call fedora_system_status");

    assert_ne!(result.is_error, Some(true));
    assert_eq!(structured(&result)["result"]["hostname"], "baileyai");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn fedora_service_control_posts_the_action_body() {
    let base_url = support::spawn(vec![MockResponse::ok("{}")]);
    let server = HomelabServer::new(None, None, Some(fedora_client(base_url)));
    let client = connect(server).await;

    let result = client
        .call_tool(call(
            "fedora_service_control",
            serde_json::json!({ "name": "ollama.service", "action": "restart" }),
        ))
        .await
        .expect("call fedora_service_control");

    assert_ne!(result.is_error, Some(true));

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn a_disallowed_fedora_unit_surfaces_as_a_protocol_error() {
    let base_url = support::spawn(vec![MockResponse::status(
        400,
        "Bad Request",
        r#"{"error":"unit 'sshd.service' is not in the allowlist"}"#,
    )]);
    let server = HomelabServer::new(None, None, Some(fedora_client(base_url)));
    let client = connect(server).await;

    let err = client
        .call_tool(call(
            "fedora_service_control",
            serde_json::json!({ "name": "sshd.service", "action": "stop" }),
        ))
        .await
        .expect_err("a unit outside the agent's allowlist should fail");

    assert!(
        err.to_string().contains("not in the allowlist"),
        "unexpected error: {err}"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn fedora_dnf_install_returns_the_task_id() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"{"task_id":"task-1"}"#)]);
    let server = HomelabServer::new(None, None, Some(fedora_client(base_url)));
    let client = connect(server).await;

    let result = client
        .call_tool(call(
            "fedora_dnf_install",
            serde_json::json!({ "packages": ["htop"] }),
        ))
        .await
        .expect("call fedora_dnf_install");

    assert_ne!(result.is_error, Some(true));
    assert_eq!(structured(&result)["result"]["task_id"], "task-1");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn an_unconfigured_fedora_backend_fails_with_a_clear_error() {
    let client = connect(HomelabServer::new(None, None, None)).await;

    let err = client
        .call_tool(call("fedora_system_status", serde_json::json!({})))
        .await
        .expect_err("fedora_system_status should fail when Fedora isn't configured");

    assert!(
        err.to_string().contains("Fedora is not configured"),
        "unexpected error: {err}"
    );

    client.cancel().await.expect("cancel");
}

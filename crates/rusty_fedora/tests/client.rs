use rusty_wiremock::canned as support;

use rusty_fedora::{FedoraAgentClient, FedoraAgentConfig, Priority, ServiceAction, UnitType};
use support::MockResponse;

fn client(base_url: String) -> FedoraAgentClient {
    FedoraAgentClient::new(FedoraAgentConfig {
        base_url,
        timeout: None,
    })
}

#[tokio::test]
async fn system_status_returns_the_body_as_is() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"hostname":"baileyai","uptime_seconds":12345}"#,
    )]);

    let status = client(base_url).system_status().await.expect("status");

    assert_eq!(status["hostname"], "baileyai");
}

#[tokio::test]
async fn list_services_with_no_filter_hits_the_bare_path() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"[{"name":"ollama.service","load_state":"loaded","active_state":"active","sub_state":"running"}]"#,
    )]);

    let services = client(base_url)
        .list_services(None)
        .await
        .expect("list_services");

    assert_eq!(services[0]["name"], "ollama.service");
}

#[tokio::test]
async fn list_services_with_a_filter_passes_the_unit_type() {
    let base_url = support::spawn(vec![MockResponse::ok("[]")]);

    let services = client(base_url)
        .list_services(Some(UnitType::Timer))
        .await
        .expect("list_services");

    assert!(services.as_array().expect("array").is_empty());
}

#[tokio::test]
async fn service_control_posts_the_action_body() {
    let base_url = support::spawn(vec![MockResponse::ok("{}")]);

    client(base_url)
        .service_control("ollama.service", ServiceAction::Restart)
        .await
        .expect("service_control");
}

#[tokio::test]
async fn read_journal_with_every_filter_set() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"[{"line":"hello"}]"#)]);

    let lines = client(base_url)
        .read_journal(
            Some("ollama.service"),
            Some(50),
            Some("1 hour ago"),
            Some(Priority::Err),
        )
        .await
        .expect("read_journal");

    assert_eq!(lines[0]["line"], "hello");
}

#[tokio::test]
async fn dnf_install_returns_the_task_id() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"{"task_id":"task-1"}"#)]);

    let result = client(base_url)
        .dnf_install(&["htop".to_string()])
        .await
        .expect("dnf_install");

    assert_eq!(result["task_id"], "task-1");
}

#[tokio::test]
async fn task_status_returns_the_body_as_is() {
    let base_url = support::spawn(vec![MockResponse::ok(
        r#"{"id":"task-1","state":"succeeded","exit_code":0}"#,
    )]);

    let status = client(base_url)
        .task_status("task-1")
        .await
        .expect("task_status");

    assert_eq!(status["state"], "succeeded");
}

#[tokio::test]
async fn read_config_passes_the_path_as_a_query_parameter() {
    let base_url = support::spawn(vec![MockResponse::ok(r#"{"content":"Hello=World\n"}"#)]);

    let result = client(base_url)
        .read_config("/etc/systemd/system/ollama.service.d/override.conf")
        .await
        .expect("read_config");

    assert_eq!(result["content"], "Hello=World\n");
}

#[tokio::test]
async fn write_config_puts_the_full_body() {
    let base_url = support::spawn(vec![MockResponse::ok("{}")]);

    client(base_url)
        .write_config(
            "/etc/systemd/system/ollama.service.d/override.conf",
            "Hello=World\n",
            true,
        )
        .await
        .expect("write_config");
}

#[tokio::test]
async fn an_agent_error_surfaces_the_status_and_body() {
    let base_url = support::spawn(vec![MockResponse::status(
        400,
        "Bad Request",
        r#"{"error":"unit 'sshd.service' is not in the allowlist"}"#,
    )]);

    let err = client(base_url)
        .service_control("sshd.service", ServiceAction::Stop)
        .await
        .expect_err("disallowed unit should fail");

    assert!(err.to_string().contains("not in the allowlist"));
}

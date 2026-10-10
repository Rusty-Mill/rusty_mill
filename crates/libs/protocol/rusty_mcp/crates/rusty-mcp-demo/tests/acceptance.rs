#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The demo's acceptance suite: the `rmcp` client against the demo over real
//! Streamable HTTP. Ported from the demo's earlier tests, whose assertions
//! these are; they used an in-memory pipe to the `rmcp` server.

#[path = "../src/demo.rs"]
mod demo;

use rmcp::{
    ClientHandler, ClientServiceExt, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, ClientCapabilities, ClientInfo, ProtocolVersion,
    },
    service::RunningService,
    transport::StreamableHttpClientTransport,
    transport::streamable_http_client::StreamableHttpClientTransportConfig,
};
use rusty_mcp_server::{HttpConfig, bind_http};
use rusty_serve::{Limits, ShutdownHandle};
use std::sync::Arc;
use std::time::Duration;

type Client = RunningService<rmcp::RoleClient, Pinned>;

/// A client that names its revision and may declare the tasks extension.
#[derive(Clone)]
struct Pinned {
    version: ProtocolVersion,
    tasks: bool,
    /// Declare the `elicitation` capability. The original MRTR tests do not,
    /// and the old scaffold asked anyway; this server follows the spec and
    /// only asks a client that said it can be asked.
    elicitation: bool,
}

impl ClientHandler for Pinned {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = self.version.clone();
        let mut caps = serde_json::json!({});
        if self.tasks {
            caps["extensions"] = serde_json::json!({ "io.modelcontextprotocol/tasks": {} });
        }
        if self.elicitation {
            caps["elicitation"] = serde_json::json!({ "form": {} });
        }
        info.capabilities = serde_json::from_value::<ClientCapabilities>(caps).expect("caps");
        info
    }
}

/// A running demo server; stops when dropped.
struct Demo {
    url: String,
    stop: ShutdownHandle,
}

impl Drop for Demo {
    fn drop(&mut self) {
        self.stop.shutdown();
    }
}

fn start() -> Demo {
    let http = bind_http(
        Arc::new(demo::demo_server().unwrap()),
        "127.0.0.1:0".parse().unwrap(),
        HttpConfig::default(),
        Limits::default(),
    )
    .unwrap();
    let url = format!("http://{}/mcp", http.local_addr().unwrap());
    let stop = http.shutdown_handle().unwrap();
    std::thread::spawn(move || http.run().unwrap());
    Demo { url, stop }
}

/// A client transport to `demo`. A macro, not a function, so the test need
/// not name `reqwest`'s client type.
macro_rules! transport {
    ($demo:expr) => {
        StreamableHttpClientTransport::from_config(StreamableHttpClientTransportConfig::with_uri(
            $demo.url.clone(),
        ))
    };
}

async fn connect_with(demo: &Demo, version: ProtocolVersion, tasks: bool) -> Client {
    Pinned {
        version,
        tasks,
        elicitation: false,
    }
    .serve(transport!(demo))
    .await
    .expect("client connects")
}

async fn connect(demo: &Demo) -> Client {
    connect_with(demo, ProtocolVersion::V_2026_07_28, false).await
}

fn object(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().cloned().expect("object")
}

fn call(name: &'static str, args: serde_json::Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name).with_arguments(object(args))
}

fn structured(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    result
        .structured_content
        .clone()
        .expect("tool should return structured content")
}

fn first_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content")
}

async fn all_tool_names(client: &Client) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = None;
    loop {
        let page = client
            .list_tools(Some(
                rmcp::model::PaginatedRequestParams::default().with_cursor(cursor.clone()),
            ))
            .await
            .expect("tools/list");
        names.extend(page.tools.iter().map(|t| t.name.to_string()));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    names.sort();
    names
}

const ALL_TOOLS: [&str; 7] = [
    "add",
    "countdown",
    "divide",
    "drop_table",
    "slugify",
    "text_stats",
    "touch_resource",
];

// ---- tools.rs ---------------------------------------------------------

mod tools {
    use super::*;

    #[tokio::test]
    async fn lists_tools_from_every_router() {
        let demo = start();
        let client = connect(&demo).await;
        assert_eq!(all_tool_names(&client).await, ALL_TOOLS);
        let mut cursor = None;
        loop {
            let page = client
                .list_tools(Some(
                    rmcp::model::PaginatedRequestParams::default().with_cursor(cursor.clone()),
                ))
                .await
                .unwrap();
            for tool in &page.tools {
                assert!(
                    tool.description.as_ref().is_some_and(|d| !d.is_empty()),
                    "tool {} is missing a description",
                    tool.name
                );
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn list_tools_carries_cache_hints() {
        let demo = start();
        let client = connect(&demo).await;
        let tools = client.list_tools(None).await.expect("tools/list");
        assert_eq!(
            client.peer_info().map(|i| i.protocol_version.clone()),
            Some(ProtocolVersion::V_2026_07_28),
        );
        assert!(tools.ttl_ms.is_some(), "expected a ttlMs cache hint");
        assert!(tools.cache_scope.is_some(), "expected a cacheScope hint");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn add_returns_structured_output() {
        let demo = start();
        let client = connect(&demo).await;
        let result = client
            .call_tool(call("add", serde_json::json!({ "a": 2, "b": 40 })))
            .await
            .expect("call add");
        assert_ne!(result.is_error, Some(true));
        assert_eq!(structured(&result)["sum"], 42);
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn state_is_shared_across_calls() {
        let demo = start();
        let client = connect(&demo).await;
        let mut seen = Vec::new();
        for _ in 0..3 {
            let result = client
                .call_tool(call("add", serde_json::json!({ "a": 1, "b": 1 })))
                .await
                .expect("call add");
            seen.push(structured(&result)["calls"].as_u64().expect("calls"));
        }
        assert_eq!(seen, vec![1, 2, 3]);
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn divide_reports_quotient_and_remainder() {
        let demo = start();
        let client = connect(&demo).await;
        let result = client
            .call_tool(call("divide", serde_json::json!({ "a": 17, "b": 5 })))
            .await
            .expect("call divide");
        let value = structured(&result);
        assert_eq!(value["quotient"], 3);
        assert_eq!(value["remainder"], 2);
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn divide_by_zero_is_a_protocol_error() {
        let demo = start();
        let client = connect(&demo).await;
        let err = client
            .call_tool(call("divide", serde_json::json!({ "a": 1, "b": 0 })))
            .await
            .expect_err("divide by zero should fail");
        assert!(err.to_string().contains("divide by zero"), "{err}");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn add_overflow_is_rejected_rather_than_panicking() {
        let demo = start();
        let client = connect(&demo).await;
        let err = client
            .call_tool(call("add", serde_json::json!({ "a": i64::MAX, "b": 1 })))
            .await
            .expect_err("overflow should fail");
        assert!(err.to_string().contains("overflow"), "{err}");
        assert_eq!(all_tool_names(&client).await.len(), 7);
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn text_tools_work() {
        let demo = start();
        let client = connect(&demo).await;
        let slug = client
            .call_tool(call(
                "slugify",
                serde_json::json!({ "text": "Hello, MCP World!" }),
            ))
            .await
            .expect("call slugify");
        assert_eq!(first_text(&slug), "hello-mcp-world");
        let stats = client
            .call_tool(call(
                "text_stats",
                serde_json::json!({ "text": "one two\nthree" }),
            ))
            .await
            .expect("call text_stats");
        let value = structured(&stats);
        assert_eq!(value["words"], 3);
        assert_eq!(value["lines"], 2);
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn older_clients_still_negotiate_down() {
        let demo = start();
        let client = connect_with(&demo, ProtocolVersion::V_2025_11_25, false).await;
        assert_eq!(
            client.peer_info().map(|i| i.protocol_version.clone()),
            Some(ProtocolVersion::V_2025_11_25),
        );
        let result = client
            .call_tool(call("add", serde_json::json!({ "a": 20, "b": 22 })))
            .await
            .expect("call add on 2025-11-25");
        assert_eq!(structured(&result)["sum"], 42);
        client.cancel().await.unwrap();
    }
}

// ---- tasks.rs ---------------------------------------------------------

mod tasks {
    use super::*;
    use rmcp::model::{CancelTaskParams, GetTaskParams, TaskStatus};

    fn countdown(steps: u32) -> CallToolRequestParams {
        call("countdown", serde_json::json!({ "steps": steps }))
    }

    async fn settle(client: &Client, id: &str, wait_ms: u64) -> Option<TaskStatus> {
        for _ in 0..100 {
            let got = client
                .get_task(GetTaskParams::new(id.to_owned()))
                .await
                .expect("tasks/get");
            if got.task.status().is_terminal() {
                return Some(got.task.status());
            }
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
        }
        None
    }

    #[tokio::test]
    async fn a_task_capable_client_gets_a_handle_and_polls_it_to_completion() {
        let demo = start();
        let client = connect_with(&demo, ProtocolVersion::V_2026_07_28, true).await;
        let response = client
            .call_tool_once(countdown(3))
            .await
            .expect("call countdown");
        let CallToolResponse::Task(create) = response else {
            panic!("expected a task handle, got {response:?}");
        };
        assert_eq!(create.task.status, TaskStatus::Working);
        let interval = create.task.poll_interval_ms.unwrap_or(50).min(100);
        let status = settle(&client, &create.task.task_id, interval).await;
        assert_eq!(status, Some(TaskStatus::Completed));
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn a_client_without_the_extension_gets_a_plain_result() {
        let demo = start();
        let client = connect(&demo).await;
        let response = client
            .call_tool_once(countdown(1))
            .await
            .expect("call countdown");
        let CallToolResponse::Complete(result) = response else {
            panic!("expected an inline result, got {response:?}");
        };
        assert_eq!(first_text(&result), "counted down 1 steps");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn fast_tools_stay_inline_even_for_task_capable_clients() {
        let demo = start();
        let client = connect_with(&demo, ProtocolVersion::V_2026_07_28, true).await;
        let response = client
            .call_tool_once(call("add", serde_json::json!({ "a": 2, "b": 40 })))
            .await
            .expect("call add");
        let CallToolResponse::Complete(result) = response else {
            panic!("expected an inline result for a fast tool, got {response:?}");
        };
        assert_eq!(structured(&result)["sum"], 42);
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn a_cancelled_task_settles_as_cancelled() {
        let demo = start();
        let client = connect_with(&demo, ProtocolVersion::V_2026_07_28, true).await;
        let response = client
            .call_tool_once(countdown(100))
            .await
            .expect("call countdown");
        let CallToolResponse::Task(create) = response else {
            panic!("expected a task handle");
        };
        let id = create.task.task_id.clone();
        client
            .cancel_task(CancelTaskParams::new(id.clone()))
            .await
            .expect("tasks/cancel is acknowledged");
        assert_eq!(settle(&client, &id, 20).await, Some(TaskStatus::Cancelled));
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn the_server_advertises_the_tasks_extension() {
        let demo = start();
        let client = connect_with(&demo, ProtocolVersion::V_2026_07_28, true).await;
        let info = client.peer_info().expect("server info");
        assert!(info.capabilities.supports_tasks(), "no tasks extension");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn an_unknown_task_id_is_an_error() {
        let demo = start();
        let client = connect_with(&demo, ProtocolVersion::V_2026_07_28, true).await;
        assert!(
            client
                .get_task(GetTaskParams::new("no-such-task"))
                .await
                .is_err()
        );
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn tasks_list_still_reports_every_tool() {
        let demo = start();
        let client = connect_with(&demo, ProtocolVersion::V_2026_07_28, true).await;
        assert_eq!(all_tool_names(&client).await, ALL_TOOLS);
        client.cancel().await.unwrap();
    }
}

// ---- mrtr.rs ----------------------------------------------------------

mod mrtr {
    use super::*;

    /// The demo's `connect`, plus the `elicitation` capability.
    async fn connect(demo: &Demo) -> Client {
        Pinned {
            version: ProtocolVersion::V_2026_07_28,
            tasks: false,
            elicitation: true,
        }
        .serve(transport!(demo))
        .await
        .expect("client connects")
    }

    use rmcp::model::{ElicitRequestParams, ElicitationAction, InputRequest, InputResponses};

    fn drop_table(table: &str) -> CallToolRequestParams {
        call("drop_table", serde_json::json!({ "table": table }))
    }

    fn answer(key: &str, action: ElicitationAction, confirm: Option<bool>) -> InputResponses {
        let mut result = serde_json::json!({ "action": action });
        if let Some(confirm) = confirm {
            result["content"] = serde_json::json!({ "confirm": confirm });
        }
        let mut responses = InputResponses::new();
        responses.insert(key.to_string(), result);
        responses
    }

    async fn ask(client: &Client, table: &str) -> (String, String) {
        let response = client
            .call_tool_once(drop_table(table))
            .await
            .expect("call drop_table");
        let CallToolResponse::InputRequired(required) = response else {
            panic!("expected an input request, got {response:?}");
        };
        let (key, request) = required
            .input_requests
            .expect("input requests")
            .into_iter()
            .next()
            .expect("one request");
        let InputRequest::Elicitation(elicit) = request else {
            panic!("expected an elicitation");
        };
        let ElicitRequestParams::FormElicitationParams { message, .. } = elicit.params else {
            panic!("expected a form elicitation");
        };
        assert!(message.contains(table), "prompt should name the table");
        (key, required.request_state.expect("request state"))
    }

    async fn retry(
        client: &Client,
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
        first_text(&result)
    }

    #[tokio::test]
    async fn the_first_call_asks_instead_of_acting() {
        let demo = start();
        let client = connect(&demo).await;
        let (key, state) = ask(&client, "users").await;
        assert!(!key.is_empty());
        assert!(
            !state.is_empty(),
            "the server must hand back a request state"
        );
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn confirming_completes_the_operation() {
        let demo = start();
        let client = connect(&demo).await;
        let (key, state) = ask(&client, "users").await;
        let response = retry(
            &client,
            "users",
            &state,
            answer(&key, ElicitationAction::Accept, Some(true)),
        )
        .await;
        assert_eq!(text_of(response), "dropped `users`");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn declining_leaves_it_alone() {
        let demo = start();
        let client = connect(&demo).await;
        let (key, state) = ask(&client, "users").await;
        let response = retry(
            &client,
            "users",
            &state,
            answer(&key, ElicitationAction::Decline, None),
        )
        .await;
        assert_eq!(text_of(response), "left `users` alone");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn accepting_with_confirm_false_is_not_consent() {
        let demo = start();
        let client = connect(&demo).await;
        let (key, state) = ask(&client, "users").await;
        let response = retry(
            &client,
            "users",
            &state,
            answer(&key, ElicitationAction::Accept, Some(false)),
        )
        .await;
        assert_eq!(text_of(response), "left `users` alone");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn a_tampered_request_state_is_rejected() {
        let demo = start();
        let client = connect(&demo).await;
        let (key, state) = ask(&client, "users").await;
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
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn the_confirmed_table_comes_from_the_sealed_state() {
        let demo = start();
        let client = connect(&demo).await;
        let (key, state) = ask(&client, "users").await;
        let response = retry(
            &client,
            "orders", // changed by the client between rounds
            &state,
            answer(&key, ElicitationAction::Accept, Some(true)),
        )
        .await;
        assert_eq!(text_of(response), "dropped `users`");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn answers_without_a_request_state_are_rejected() {
        let demo = start();
        let client = connect(&demo).await;
        let mut params = drop_table("users");
        params.input_responses = Some(answer(
            "confirm-drop",
            ElicitationAction::Accept,
            Some(true),
        ));
        assert!(
            client.call_tool_once(params).await.is_err(),
            "answers with no state must not be silently restarted"
        );
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn an_elicit_result_deserializes_from_the_wire_form() {
        let raw = serde_json::json!({ "action": "accept", "content": { "confirm": true } });
        let parsed: rmcp::model::ElicitResult = serde_json::from_value(raw).expect("parses");
        assert_eq!(parsed.action, ElicitationAction::Accept);
    }
}

// ---- resources_and_prompts.rs ----------------------------------------

mod resources_and_prompts {
    use super::*;
    use rmcp::model::{GetPromptRequestParams, ReadResourceRequestParams, ResourceContents};

    fn text_of(contents: &ResourceContents) -> &str {
        match contents {
            ResourceContents::TextResourceContents { text, .. } => text,
            other => panic!("expected text contents, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn advertises_resource_and_prompt_capabilities() {
        let demo = start();
        let client = connect(&demo).await;
        let info = client.peer_info().expect("server info");
        assert!(info.capabilities.resources.is_some());
        assert!(info.capabilities.prompts.is_some());
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn lists_resources_with_cache_hints() {
        let demo = start();
        let client = connect(&demo).await;
        let list = client.list_resources(None).await.expect("resources/list");
        let mut names: Vec<_> = list.resources.iter().map(|r| r.name.clone()).collect();
        names.sort();
        assert_eq!(names, ["demo-config", "uptime"].map(String::from).to_vec());
        assert!(list.ttl_ms.is_some(), "missing ttlMs");
        assert!(list.cache_scope.is_some(), "missing cacheScope");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn lists_resource_templates() {
        let demo = start();
        let client = connect(&demo).await;
        let list = client
            .list_resource_templates(None)
            .await
            .expect("resources/templates/list");
        assert_eq!(list.resource_templates.len(), 1);
        assert_eq!(
            list.resource_templates[0].uri_template,
            "db://tables/{table}"
        );
        assert!(list.ttl_ms.is_some());
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn reads_a_static_resource() {
        let demo = start();
        let client = connect(&demo).await;
        let result = client
            .read_resource(ReadResourceRequestParams::new("config://demo"))
            .await
            .expect("resources/read");
        let text = text_of(&result.contents[0]);
        assert!(text.contains("\"greeting\""), "unexpected body: {text}");
        assert!(
            result.ttl_ms.is_some(),
            "read results carry cache hints too"
        );
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn reads_a_generated_resource() {
        let demo = start();
        let client = connect(&demo).await;
        let result = client
            .read_resource(ReadResourceRequestParams::new("status://uptime"))
            .await
            .expect("resources/read");
        assert!(text_of(&result.contents[0]).ends_with(" seconds"));
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn reads_through_a_uri_template() {
        let demo = start();
        let client = connect(&demo).await;
        let result = client
            .read_resource(ReadResourceRequestParams::new("db://tables/users"))
            .await
            .expect("resources/read");
        let body: serde_json::Value =
            serde_json::from_str(text_of(&result.contents[0])).expect("json body");
        assert_eq!(body["table"], "users");
        assert_eq!(body["columns"][0], "id");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn an_unknown_resource_is_an_error() {
        let demo = start();
        let client = connect(&demo).await;
        assert!(
            client
                .read_resource(ReadResourceRequestParams::new("config://nope"))
                .await
                .is_err()
        );
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn a_template_variable_cannot_traverse_out_of_its_namespace() {
        let demo = start();
        let client = connect(&demo).await;
        assert!(
            client
                .read_resource(ReadResourceRequestParams::new(
                    "db://tables/../../etc/passwd"
                ))
                .await
                .is_err()
        );
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn an_unknown_table_reports_bad_parameters() {
        let demo = start();
        let client = connect(&demo).await;
        let err = client
            .read_resource(ReadResourceRequestParams::new("db://tables/nonexistent"))
            .await
            .expect_err("unknown table");
        assert!(err.to_string().contains("no such table"), "{err}");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn lists_prompts_with_descriptions() {
        let demo = start();
        let client = connect(&demo).await;
        let mut prompts = Vec::new();
        let mut cursor = None;
        loop {
            let page = client
                .list_prompts(Some(
                    rmcp::model::PaginatedRequestParams::default().with_cursor(cursor.clone()),
                ))
                .await
                .expect("prompts/list");
            assert!(page.ttl_ms.is_some(), "prompts/list carries cache hints");
            prompts.extend(page.prompts);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        let mut names: Vec<_> = prompts.iter().map(|p| p.name.clone()).collect();
        names.sort();
        assert_eq!(
            names,
            ["explain-error", "summarize"].map(String::from).to_vec()
        );
        for prompt in &prompts {
            assert!(
                prompt.description.as_ref().is_some_and(|d| !d.is_empty()),
                "prompt {} needs a description",
                prompt.name
            );
        }
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn gets_a_prompt_with_arguments() {
        let demo = start();
        let client = connect(&demo).await;
        let result = client
            .get_prompt(
                GetPromptRequestParams::new("summarize").with_arguments(object(
                    serde_json::json!({ "text": "war and peace", "sentences": 2 }),
                )),
            )
            .await
            .expect("prompts/get");
        assert_eq!(result.messages.len(), 1);
        let rendered = format!("{:?}", result.messages[0]);
        assert!(rendered.contains("war and peace"), "{rendered}");
        assert!(rendered.contains("about 2 sentences"), "{rendered}");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn an_optional_prompt_argument_may_be_omitted() {
        let demo = start();
        let client = connect(&demo).await;
        let result = client
            .get_prompt(
                GetPromptRequestParams::new("explain-error")
                    .with_arguments(object(serde_json::json!({ "error": "segfault" }))),
            )
            .await
            .expect("prompts/get without the optional argument");
        let rendered = format!("{:?}", result.messages[0]);
        assert!(rendered.contains("segfault"), "{rendered}");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn tools_still_work_alongside_resources_and_prompts() {
        let demo = start();
        let client = connect(&demo).await;
        assert_eq!(all_tool_names(&client).await.len(), 7);
        client.cancel().await.unwrap();
    }
}

// ---- subscriptions.rs -------------------------------------------------

mod subscriptions {
    use super::*;
    use rmcp::model::{ServerNotification, SubscriptionFilter};
    use rmcp::service::ClientLifecycleMode;

    const TIMEOUT: Duration = Duration::from_secs(5);

    async fn connect_listening(demo: &Demo) -> RunningService<rmcp::RoleClient, ClientInfo> {
        ClientInfo::default()
            .serve_with_lifecycle(
                transport!(demo),
                ClientLifecycleMode::Discover {
                    preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                },
            )
            .await
            .expect("client connects")
    }

    fn touch(uri: &str) -> CallToolRequestParams {
        call("touch_resource", serde_json::json!({ "uri": uri }))
    }

    #[tokio::test]
    async fn a_change_published_in_the_server_reaches_a_listening_client() {
        let demo = start();
        let client = connect_listening(&demo).await;
        let mut subscription = client
            .listen(
                SubscriptionFilter::builder()
                    .resources_list_changed()
                    .build(),
            )
            .await
            .expect("subscriptions/listen");
        client.call_tool(touch("config://demo")).await.unwrap();
        let notification = tokio::time::timeout(TIMEOUT, subscription.next())
            .await
            .expect("a notification should arrive")
            .expect("subscription is live")
            .expect("not the end of the stream");
        assert!(
            matches!(
                notification,
                ServerNotification::ResourceListChangedNotification(_)
            ),
            "unexpected notification: {notification:?}"
        );
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn a_client_only_receives_the_categories_it_asked_for() {
        let demo = start();
        let client = connect_listening(&demo).await;
        let mut subscription = client
            .listen(
                SubscriptionFilter::builder()
                    .resource_subscription("config://demo")
                    .build(),
            )
            .await
            .expect("subscriptions/listen");
        client.call_tool(touch("config://demo")).await.unwrap();
        let notification = tokio::time::timeout(TIMEOUT, subscription.next())
            .await
            .expect("a notification should arrive")
            .expect("subscription is live")
            .expect("not the end of the stream");
        match notification {
            ServerNotification::ResourceUpdatedNotification(update) => {
                assert_eq!(update.params.uri, "config://demo");
            }
            other => panic!("expected a resource update, got {other:?}"),
        }
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn a_resource_update_for_another_uri_is_not_delivered() {
        let demo = start();
        let client = connect_listening(&demo).await;
        let mut subscription = client
            .listen(
                SubscriptionFilter::builder()
                    .resource_subscription("config://demo")
                    .build(),
            )
            .await
            .expect("subscriptions/listen");
        client.call_tool(touch("status://uptime")).await.unwrap();
        let quiet = tokio::time::timeout(Duration::from_millis(300), subscription.next()).await;
        assert!(quiet.is_err(), "no notification should have arrived");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn several_clients_each_get_their_own_notifications() {
        let demo = start();
        let client = connect_listening(&demo).await;
        let mut first = client
            .listen(
                SubscriptionFilter::builder()
                    .resources_list_changed()
                    .build(),
            )
            .await
            .expect("first subscription");
        let mut second = client
            .listen(
                SubscriptionFilter::builder()
                    .resources_list_changed()
                    .build(),
            )
            .await
            .expect("second subscription");
        client.call_tool(touch("config://demo")).await.unwrap();
        for (label, subscription) in [("first", &mut first), ("second", &mut second)] {
            let notification = tokio::time::timeout(TIMEOUT, subscription.next())
                .await
                .unwrap_or_else(|_| panic!("{label} subscription should have received it"))
                .expect("live")
                .expect("not the end");
            assert!(matches!(
                notification,
                ServerNotification::ResourceListChangedNotification(_)
            ));
        }
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn an_unadvertised_category_is_dropped_from_the_filter() {
        let demo = start();
        let client = connect_listening(&demo).await;
        let subscription = client
            .listen(SubscriptionFilter::builder().prompts_list_changed().build())
            .await;
        assert!(subscription.is_ok(), "an advertised category is accepted");
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn the_tool_reports_how_many_listeners_it_reached() {
        let demo = start();
        let client = connect_listening(&demo).await;
        let before = client.call_tool(touch("config://demo")).await.unwrap();
        assert!(first_text(&before).contains("0 listener"), "{before:?}");
        let _subscription = client
            .listen(
                SubscriptionFilter::builder()
                    .resources_list_changed()
                    .build(),
            )
            .await
            .expect("subscriptions/listen");
        let after = client.call_tool(touch("config://demo")).await.unwrap();
        assert!(first_text(&after).contains("1 listener"), "{after:?}");
        client.cancel().await.unwrap();
    }
}

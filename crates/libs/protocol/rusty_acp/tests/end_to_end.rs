//! End-to-end tests: the client driving a real server over HTTP.

#![cfg(all(feature = "client", feature = "server"))]

use std::time::Duration;

use futures_util::StreamExt;
use rusty_acp::{
    client::{collect_run, AcpClient, WaitOptions},
    server::{agent_fn, AcpServer, RunContext},
    types::{
        AgentManifest, AgentName, AwaitRequest, Error, ErrorCode, Event, Message, MessagePart,
        RunCreateRequest, RunMode, RunResumeRequest, RunStatus, SessionId,
    },
    AcpError,
};
use serde_json::json;

/// Spin up the full agent suite on an ephemeral port and return a client for it.
async fn start_server() -> AcpClient {
    let echo = agent_fn(
        AgentManifest::new(AgentName::new("echo").unwrap(), "Echoes the input back"),
        |ctx: RunContext| async move {
            ctx.reply_text(ctx.input_text()).await?;
            Ok(())
        },
    );

    let words = agent_fn(
        AgentManifest::new(AgentName::new("words").unwrap(), "Streams one part per word"),
        |ctx: RunContext| async move {
            let mut writer = ctx.begin_message().await?;
            for word in ctx.input_text().split_whitespace() {
                writer.push_text(word).await?;
            }
            writer.finish().await?;
            Ok(())
        },
    );

    let greeter = agent_fn(
        AgentManifest::new(AgentName::new("greeter").unwrap(), "Asks for a name, then greets"),
        |ctx: RunContext| async move {
            let resume =
                ctx.await_request(AwaitRequest::new(json!({ "question": "name?" }))).await?;
            let name = resume.as_value()["answer"].as_str().unwrap_or("stranger").to_string();
            ctx.reply_text(format!("Hello, {name}!")).await?;
            Ok(())
        },
    );

    let boom = agent_fn(
        AgentManifest::new(AgentName::new("boom").unwrap(), "Always fails"),
        |_ctx: RunContext| async move { Err(Error::server_error("model unavailable")) },
    );

    let forever = agent_fn(
        AgentManifest::new(AgentName::new("forever").unwrap(), "Never finishes on its own"),
        |ctx: RunContext| async move {
            ctx.emit_generic(json!({ "phase": "started" })).await?;
            ctx.cancelled().await;
            Ok(())
        },
    );

    let vision = agent_fn(
        AgentManifest::new(AgentName::new("vision").unwrap(), "Only accepts images")
            .with_input_content_types(["image/*"]),
        |ctx: RunContext| async move {
            ctx.reply_text("saw an image").await?;
            Ok(())
        },
    );

    // Reports what it loaded, then bumps it — so a caller can see state persist.
    let remember = agent_fn(
        AgentManifest::new(AgentName::new("remember").unwrap(), "Counts turns in session state"),
        |ctx: RunContext| async move {
            let previous: u32 = ctx.load_state().await?.unwrap_or(0);
            ctx.store_state(&(previous + 1)).await?;
            ctx.reply_text(format!("seen {previous}")).await?;
            Ok(())
        },
    );

    let artist = agent_fn(
        AgentManifest::new(AgentName::new("artist").unwrap(), "Returns artifacts")
            .with_output_content_types(["*/*"]),
        |ctx: RunContext| async move {
            ctx.reply_artifact("result.json", "application/json", r#"{"ok":true}"#).await?;
            ctx.reply_part(MessagePart::binary_artifact(
                "chart.png",
                "image/png",
                [0x89, 0x50, 0x4e],
            ))
            .await?;
            Ok(())
        },
    );

    let router = AcpServer::builder()
        .agent(echo)
        .agent(words)
        .agent(greeter)
        .agent(boom)
        .agent(forever)
        .agent(vision)
        .agent(remember)
        .agent(artist)
        .build()
        .expect("server builds")
        .into_router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    AcpClient::new(format!("http://{addr}")).unwrap()
}

#[tokio::test]
async fn ping_reports_the_server_is_reachable() {
    let client = start_server().await;
    client.ping().await.unwrap();
}

#[tokio::test]
async fn discovery_lists_agents_and_paginates() {
    let client = start_server().await;

    let all = client.list_all_agents().await.unwrap();
    assert_eq!(all.len(), 8);
    assert_eq!(all[0].name.as_str(), "echo");

    let first_page = client.list_agents(Some(2), Some(0)).await.unwrap();
    assert_eq!(first_page.len(), 2);
    let second_page = client.list_agents(Some(2), Some(2)).await.unwrap();
    assert_eq!(second_page.len(), 2);
    assert_ne!(first_page[0].name, second_page[0].name);

    // The spec's default page size is 10.
    assert_eq!(client.list_agents(None, None).await.unwrap().len(), 8);
}

#[tokio::test]
async fn get_agent_returns_a_manifest_or_not_found() {
    let client = start_server().await;

    let manifest = client.get_agent("echo").await.unwrap();
    assert_eq!(manifest.description, "Echoes the input back");

    let error = client.get_agent("missing").await.unwrap_err();
    assert!(error.is_not_found(), "expected not_found, got {error}");
}

#[tokio::test]
async fn sync_run_blocks_until_the_agent_completes() {
    let client = start_server().await;

    let run = client.run_sync("echo", [Message::user("Hello, ACP!")]).await.unwrap();

    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.output_text(), "Hello, ACP!");
    assert_eq!(run.output.len(), 1);
    assert_eq!(run.output[0].role.agent_name(), Some("echo"));
    assert!(run.finished_at.is_some());
    assert!(run.error.is_none());
}

#[tokio::test]
async fn async_run_returns_immediately_and_can_be_polled() {
    let client = start_server().await;

    let started = client.run_async("echo", [Message::user("later")]).await.unwrap();
    assert!(!started.status.is_terminal() || started.status == RunStatus::Completed);

    let finished = client
        .wait_for_run(started.run_id, WaitOptions::default().poll_every(Duration::from_millis(10)))
        .await
        .unwrap();
    assert_eq!(finished.run_id, started.run_id);
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(finished.output_text(), "later");
}

#[tokio::test]
async fn stream_run_delivers_events_in_order_and_ends_after_the_terminal_event() {
    let client = start_server().await;

    let mut stream = client.stream("words", [Message::user("one two three")]).await.unwrap();
    let mut types = Vec::new();
    let mut parts = Vec::new();
    while let Some(event) = stream.next().await {
        let event = event.unwrap();
        types.push(event.event_type().to_string());
        if let Event::MessagePart { part } = &event {
            parts.push(part.content.clone().unwrap_or_default());
        }
    }

    assert_eq!(parts, ["one", "two", "three"]);
    assert_eq!(types.first().map(String::as_str), Some("run.created"));
    assert_eq!(types.last().map(String::as_str), Some("run.completed"));
    assert!(types.contains(&"run.in-progress".to_string()));
    assert!(types.contains(&"message.created".to_string()));
    assert!(types.contains(&"message.completed".to_string()));

    // `message.completed` must arrive before the run terminates.
    let completed = types.iter().position(|t| t == "message.completed").unwrap();
    assert!(completed < types.len() - 1);
}

#[tokio::test]
async fn streamed_parts_are_aggregated_into_the_run_output() {
    let client = start_server().await;

    let stream = client.stream("words", [Message::user("alpha beta")]).await.unwrap();
    let run = collect_run(stream).await.unwrap();

    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.output.len(), 1);
    assert_eq!(run.output[0].parts.len(), 2);
    assert_eq!(run.output_text(), "alphabeta");
}

#[tokio::test]
async fn await_and_resume_round_trip() {
    let client = start_server().await;

    let paused = client.run_sync("greeter", [Message::user("hi")]).await.unwrap();
    assert_eq!(paused.status, RunStatus::Awaiting);
    let request = paused.await_request.as_ref().expect("await_request is set");
    assert_eq!(request.as_value()["question"], "name?");
    assert!(paused.output.is_empty());

    let resumed = client
        .resume_run(RunResumeRequest::new(
            paused.run_id,
            json!({ "answer": "Ada" }).into(),
            RunMode::Sync,
        ))
        .await
        .unwrap();

    assert_eq!(resumed.status, RunStatus::Completed);
    assert_eq!(resumed.output_text(), "Hello, Ada!");
    assert!(resumed.await_request.is_none());
}

#[tokio::test]
async fn resuming_a_run_that_is_not_awaiting_is_rejected() {
    let client = start_server().await;

    let run = client.run_sync("echo", [Message::user("done already")]).await.unwrap();
    let error = client
        .resume_run(RunResumeRequest::new(run.run_id, json!({}).into(), RunMode::Sync))
        .await
        .unwrap_err();

    assert_eq!(error.code(), Some(ErrorCode::InvalidInput));
}

#[tokio::test]
async fn streaming_a_resume_emits_the_events_that_follow() {
    let client = start_server().await;

    let paused = client.run_sync("greeter", [Message::user("hi")]).await.unwrap();
    let stream = client
        .stream_resume(RunResumeRequest::new(
            paused.run_id,
            json!({ "answer": "Grace" }).into(),
            RunMode::Stream,
        ))
        .await
        .unwrap();

    let run = collect_run(stream).await.unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.output_text(), "Hello, Grace!");
}

#[tokio::test]
async fn a_failing_agent_produces_a_failed_run_carrying_its_error() {
    let client = start_server().await;

    let run = client.run_sync("boom", [Message::user("go")]).await.unwrap();

    assert_eq!(run.status, RunStatus::Failed);
    let error = run.clone().into_result().unwrap_err();
    assert_eq!(error.code, ErrorCode::ServerError);
    assert_eq!(error.message, "model unavailable");
    assert!(run.finished_at.is_some());
}

#[tokio::test]
async fn cancelling_a_running_agent_terminates_it() {
    let client = start_server().await;

    let started = client.run_async("forever", [Message::user("hang")]).await.unwrap();
    let cancelled = client.cancel_and_wait(started.run_id, WaitOptions::default()).await.unwrap();

    assert_eq!(cancelled.status, RunStatus::Cancelled);
    assert!(cancelled.finished_at.is_some());

    let events = client.list_run_events(started.run_id).await.unwrap();
    assert!(events.iter().any(|event| matches!(event, Event::Generic { .. })));
    assert!(matches!(events.last(), Some(Event::RunCancelled { .. })));
}

#[tokio::test]
async fn cancelling_an_awaiting_run_terminates_it() {
    let client = start_server().await;

    let paused = client.run_sync("greeter", [Message::user("hi")]).await.unwrap();
    assert_eq!(paused.status, RunStatus::Awaiting);

    let cancelled = client.cancel_and_wait(paused.run_id, WaitOptions::default()).await.unwrap();
    assert_eq!(cancelled.status, RunStatus::Cancelled);
}

#[tokio::test]
async fn the_event_log_records_every_event_after_the_run_ends() {
    let client = start_server().await;

    let run = client.run_sync("words", [Message::user("a b")]).await.unwrap();
    let events = client.list_run_events(run.run_id).await.unwrap();

    let types: Vec<_> = events.iter().map(|event| event.event_type()).collect();
    assert_eq!(types.first(), Some(&"run.created"));
    assert_eq!(types.last(), Some(&"run.completed"));
    assert_eq!(types.iter().filter(|t| **t == "message.part").count(), 2);
}

#[tokio::test]
async fn sessions_accumulate_history_across_runs() {
    let client = start_server().await;
    let session_id = SessionId::new();

    for text in ["first", "second"] {
        let run = client
            .create_run(
                RunCreateRequest::new(AgentName::new("echo").unwrap(), [Message::user(text)])
                    .with_session_id(session_id),
            )
            .await
            .unwrap();
        assert_eq!(run.session_id, Some(session_id));
        assert_eq!(run.status, RunStatus::Completed);
    }

    let session = client.get_session(session_id).await.unwrap();
    assert_eq!(session.id, session_id);
    // Two runs, each contributing one input and one output message.
    assert_eq!(session.history.len(), 4);

    let messages = client.fetch_session_history(&session).await.unwrap();
    let texts: Vec<_> = messages.iter().map(|message| message.text()).collect();
    assert_eq!(texts, ["first", "first", "second", "second"]);
    assert!(messages[0].role.agent_name().is_none());
    assert_eq!(messages[1].role.agent_name(), Some("echo"));
}

#[tokio::test]
async fn an_agent_sees_the_local_history_of_its_session() {
    let history_reporter = agent_fn(
        AgentManifest::new(AgentName::new("historian").unwrap(), "Reports how much it remembers"),
        |ctx: RunContext| async move {
            ctx.reply_text(format!("{} prior messages", ctx.history().len())).await?;
            Ok(())
        },
    );
    let router = AcpServer::builder().agent(history_reporter).build().unwrap().into_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = AcpClient::new(format!("http://{addr}")).unwrap();

    let session_id = SessionId::new();
    let request = |text: &str| {
        RunCreateRequest::new(AgentName::new("historian").unwrap(), [Message::user(text)])
            .with_session_id(session_id)
    };

    let first = client.create_run(request("one")).await.unwrap();
    assert_eq!(first.output_text(), "0 prior messages");

    let second = client.create_run(request("two")).await.unwrap();
    assert_eq!(second.output_text(), "2 prior messages");
}

#[tokio::test]
async fn unknown_runs_and_sessions_report_not_found() {
    let client = start_server().await;

    let missing_run = rusty_acp::types::RunId::new();
    assert!(client.get_run(missing_run).await.unwrap_err().is_not_found());
    assert!(client.list_run_events(missing_run).await.unwrap_err().is_not_found());
    assert!(client.get_session(SessionId::new()).await.unwrap_err().is_not_found());
}

#[tokio::test]
async fn running_an_unknown_agent_reports_not_found() {
    let client = start_server().await;
    let error = client.run_sync("nope", [Message::user("hi")]).await.unwrap_err();
    assert!(error.is_not_found(), "expected not_found, got {error}");
}

#[tokio::test]
async fn input_the_agent_cannot_consume_is_rejected() {
    let client = start_server().await;

    let request = RunCreateRequest::new(
        AgentName::new("vision").unwrap(),
        [Message::new(rusty_acp::types::Role::User, [MessagePart::text("not an image")])],
    );
    let error = client.create_run(request).await.unwrap_err();
    assert_eq!(error.code(), Some(ErrorCode::InvalidInput));
}

#[tokio::test]
async fn malformed_agent_names_are_rejected_by_the_client() {
    let client = start_server().await;
    let error = client.run_sync("Not A Valid Name", [Message::user("hi")]).await.unwrap_err();
    assert_eq!(error.code(), Some(ErrorCode::InvalidInput));
}

#[tokio::test]
async fn create_run_refuses_stream_mode() {
    let client = start_server().await;
    let request = RunCreateRequest::new(AgentName::new("echo").unwrap(), [Message::user("hi")])
        .with_mode(RunMode::Stream);
    assert!(matches!(client.create_run(request).await, Err(AcpError::Protocol(_))));
}

#[tokio::test]
async fn the_client_rejects_a_base_url_without_a_scheme() {
    assert!(matches!(AcpClient::new("localhost:8000"), Err(AcpError::InvalidUrl(_))));
}

#[tokio::test]
async fn a_server_needs_at_least_one_agent() {
    assert!(AcpServer::builder().build().is_err());
}

#[tokio::test]
async fn duplicate_agent_names_are_rejected() {
    let manifest = || AgentManifest::new(AgentName::new("dup").unwrap(), "Duplicated");
    let build = AcpServer::builder()
        .agent(agent_fn(manifest(), |_| async { Ok(()) }))
        .agent(agent_fn(manifest(), |_| async { Ok(()) }))
        .build();
    assert!(build.is_err());
}

// ---------------------------------------------------------------------------
// Session state (#4)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn session_state_persists_across_runs() {
    let client = start_server().await;
    let session_id = SessionId::new();

    let request = || {
        RunCreateRequest::new(AgentName::new("remember").unwrap(), [Message::user("go")])
            .with_session_id(session_id)
    };

    assert_eq!(client.create_run(request()).await.unwrap().output_text(), "seen 0");
    assert_eq!(client.create_run(request()).await.unwrap().output_text(), "seen 1");
    assert_eq!(client.create_run(request()).await.unwrap().output_text(), "seen 2");
}

#[tokio::test]
async fn stored_state_is_reachable_through_the_session_link() {
    let client = start_server().await;
    let session_id = SessionId::new();

    client
        .create_run(
            RunCreateRequest::new(AgentName::new("remember").unwrap(), [Message::user("go")])
                .with_session_id(session_id),
        )
        .await
        .unwrap();

    let session = client.get_session(session_id).await.unwrap();
    let state_url = session.state.as_deref().expect("state link is set");
    assert!(state_url.ends_with(&format!("/session/{session_id}/state")));

    // The link resolves, and yields what the agent stored.
    let state: u32 = client.fetch_session_state(&session).await.unwrap().expect("state present");
    assert_eq!(state, 1);
}

#[tokio::test]
async fn a_session_without_state_reports_none() {
    let client = start_server().await;
    let session_id = SessionId::new();

    client
        .create_run(
            RunCreateRequest::new(AgentName::new("echo").unwrap(), [Message::user("hi")])
                .with_session_id(session_id),
        )
        .await
        .unwrap();

    let session = client.get_session(session_id).await.unwrap();
    assert!(session.state.is_none(), "no state stored, so no link");
    assert!(client.fetch_session_state::<serde_json::Value>(&session).await.unwrap().is_none());
}

#[tokio::test]
async fn storing_state_without_a_session_is_rejected() {
    let client = start_server().await;

    // No `session_id`, so there is nothing to scope state to.
    let run = client.run_sync("remember", [Message::user("go")]).await.unwrap();

    assert_eq!(run.status, RunStatus::Failed);
    let error = run.into_result().unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(error.message.contains("session"), "error should explain why: {}", error.message);
}

// ---------------------------------------------------------------------------
// Artifacts (#6)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn artifacts_round_trip_with_their_names_and_encoding() {
    let client = start_server().await;

    let run = client.run_sync("artist", [Message::user("draw")]).await.unwrap();
    assert_eq!(run.status, RunStatus::Completed);

    let parts: Vec<_> = run.output.iter().flat_map(|message| &message.parts).collect();
    assert_eq!(parts.len(), 2);

    let json = parts[0];
    assert_eq!(json.artifact_name(), Some("result.json"));
    assert_eq!(json.content_type, "application/json");
    assert_eq!(json.decoded_content().unwrap().unwrap(), br#"{"ok":true}"#);

    let png = parts[1];
    assert_eq!(png.artifact_name(), Some("chart.png"));
    assert_eq!(png.content_type, "image/png");
    // Survives the wire as base64 and decodes back to the original bytes.
    assert_eq!(png.encoding(), rusty_acp::types::ContentEncoding::Base64);
    assert_eq!(png.decoded_content().unwrap().unwrap(), vec![0x89, 0x50, 0x4e]);
}

// ---------------------------------------------------------------------------
// Open discovery (#5)
// ---------------------------------------------------------------------------

#[cfg(feature = "well-known")]
#[tokio::test]
async fn well_known_serves_the_same_manifests_as_the_agents_endpoint() {
    let client = start_server().await;

    let response = client
        .http_client()
        .get(format!("{}/.well-known/agent.yml", client.base_url()))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    assert_eq!(
        response.headers().get("content-type").and_then(|value| value.to_str().ok()),
        Some("application/yaml")
    );

    let yaml = response.text().await.unwrap();
    let parsed: rusty_acp::types::AgentsListResponse = serde_norway::from_str(&yaml).unwrap();

    // One source of truth: the YAML must match what `GET /agents` reports.
    assert_eq!(parsed.agents, client.list_all_agents().await.unwrap());
}

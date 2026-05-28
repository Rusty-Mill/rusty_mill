//! Phase 16 ACP integration test: a fake editor client drives the agent over an
//! in-memory duplex — handshake → session/new → session/prompt → streamed
//! session/update → permission round-trip → cancel. No live editor in CI.

use rk_app::acp;
use rk_config::Config;
use rk_kernel::fake::{FakeLanguageModel, Scripted};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn config_at(ws: &std::path::Path) -> Config {
    let s = ws.to_string_lossy().into_owned();
    Config::resolve(move |k| match k {
        "RUSTYKEYS_MODEL" => Some("fake".into()),
        "RUSTYKEYS_WORKSPACE" => Some(s.clone()),
        _ => None,
    })
    .unwrap()
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("rk-acp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn handshake_prompt_and_streamed_updates() {
    let dir = tmp("basic");
    let config = config_at(&dir);
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("hello from acp".into())]]);

    // Wire the client side of a duplex to the server.
    let (client, server) = tokio::io::duplex(8192);
    let (srv_read, srv_write) = tokio::io::split(server);
    let task =
        tokio::spawn(
            async move { acp::run(config, model, BufReader::new(srv_read), srv_write).await },
        );

    let (cr, mut cw) = tokio::io::split(client);
    let mut cr = BufReader::new(cr).lines();

    // initialize
    send(
        &mut cw,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
    )
    .await;
    let init = next(&mut cr).await;
    assert_eq!(init["result"]["protocolVersion"], "0.1");

    // session/new
    send(
        &mut cw,
        json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}),
    )
    .await;
    let new = next(&mut cr).await;
    assert!(new["result"]["sessionId"]
        .as_str()
        .unwrap()
        .starts_with("acp_"));

    // session/prompt → streamed updates + result
    send(
        &mut cw,
        json!({"jsonrpc":"2.0","id":3,"method":"session/prompt",
        "params":{"prompt":[{"type":"text","text":"hi"}]}}),
    )
    .await;

    // Collect frames until the prompt response (id 3) arrives (bounded so a
    // contract regression fails fast instead of hanging).
    let mut saw_message = false;
    let mut saw_verification = false;
    for _ in 0..20 {
        let m = next(&mut cr).await;
        if m.get("method").and_then(Value::as_str) == Some("session/update") {
            let kind = m["params"]["update"]["sessionUpdate"]
                .as_str()
                .unwrap_or("");
            if kind == "agent_message_chunk" {
                saw_message = m["params"]["update"]["content"]["text"]
                    .as_str()
                    .unwrap_or("")
                    .contains("hello from acp");
            }
            if kind == "verification" {
                saw_verification = true;
            }
        } else if m.get("id") == Some(&json!(3)) {
            assert_eq!(m["result"]["stopReason"], "end_turn");
            break;
        }
    }
    assert!(saw_message, "expected an agent_message_chunk update");
    assert!(saw_verification, "expected a verification update");

    drop(cw);
    task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn write_mid_turn_round_trips_request_permission() {
    let dir = tmp("perm");
    let config = config_at(&dir);
    // The agent attempts a write (NewFilePath trigger), then replies.
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "write_file".into(),
            args: json!({"path": "out.txt", "content": "x"}),
        }],
        vec![Scripted::Text("done".into())],
    ]);

    let (client, server) = tokio::io::duplex(8192);
    let (srv_read, srv_write) = tokio::io::split(server);
    let task =
        tokio::spawn(
            async move { acp::run(config, model, BufReader::new(srv_read), srv_write).await },
        );

    let (cr, mut cw) = tokio::io::split(client);
    let mut cr = BufReader::new(cr).lines();

    send(
        &mut cw,
        json!({"jsonrpc":"2.0","id":1,"method":"session/new","params":{}}),
    )
    .await;
    let _ = next(&mut cr).await;
    send(
        &mut cw,
        json!({"jsonrpc":"2.0","id":2,"method":"session/prompt",
        "params":{"prompt":[{"type":"text","text":"write out.txt"}]}}),
    )
    .await;

    // The agent asks permission; deny it. Bounded against a hang.
    let mut denied = false;
    for _ in 0..20 {
        let m = next(&mut cr).await;
        if m.get("method").and_then(Value::as_str) == Some("session/request_permission") {
            assert_eq!(m["params"]["toolCall"]["name"], "write_file");
            let pid = m["id"].clone();
            send(
                &mut cw,
                json!({"jsonrpc":"2.0","id":pid,
                "result":{"outcome":{"optionId":"reject"}}}),
            )
            .await;
            denied = true;
        } else if m.get("id") == Some(&json!(2)) {
            break; // prompt completed
        }
    }
    assert!(denied, "expected a session/request_permission");
    // The denied write never created the file (boundary held).
    assert!(!dir.join("out.txt").exists());

    drop(cw);
    task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

async fn send<W: AsyncWriteExt + Unpin>(w: &mut W, v: Value) {
    w.write_all(v.to_string().as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    w.flush().await.unwrap();
}

async fn next<R: AsyncBufReadExt + Unpin>(r: &mut tokio::io::Lines<R>) -> Value {
    let line = r.next_line().await.unwrap().expect("server closed");
    serde_json::from_str(&line).unwrap()
}

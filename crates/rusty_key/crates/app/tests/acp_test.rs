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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fs_write_out_of_workspace_is_blocked_before_reaching_client() {
    let dir = tmp("fsblock");
    let config = config_at(&dir);
    // The agent attempts an out-of-workspace fs write, then replies.
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "fs_write_text_file".into(),
            args: json!({"path": "/etc/evil", "content": "x"}),
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

    // Advertise fs read+write capability so the shims register.
    send(
        &mut cw,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{"clientCapabilities":{"fs":{"readTextFile":true,"writeTextFile":true}}}}),
    )
    .await;
    let _ = next(&mut cr).await;
    send(
        &mut cw,
        json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}),
    )
    .await;
    let _ = next(&mut cr).await;
    send(
        &mut cw,
        json!({"jsonrpc":"2.0","id":3,"method":"session/prompt",
        "params":{"prompt":[{"type":"text","text":"write /etc/evil"}]}}),
    )
    .await;

    // The boundary must hold: no fs/write_text_file request ever reaches us.
    let mut completed = false;
    for _ in 0..20 {
        let m = next(&mut cr).await;
        assert_ne!(
            m.get("method").and_then(Value::as_str),
            Some("fs/write_text_file"),
            "out-of-workspace write must be blocked before reaching the client"
        );
        if m.get("id") == Some(&json!(3)) {
            completed = true;
            break;
        }
    }
    assert!(completed, "expected the prompt to complete");

    drop(cw);
    task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fs_read_bridges_to_client_capability() {
    let dir = tmp("fsread");
    let config = config_at(&dir);
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "fs_read_text_file".into(),
            args: json!({"path": "src/a.rs"}),
        }],
        vec![Scripted::Text("read it".into())],
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
        json!({"jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{"clientCapabilities":{"fs":{"readTextFile":true}}}}),
    )
    .await;
    let _ = next(&mut cr).await;
    send(
        &mut cw,
        json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}),
    )
    .await;
    let _ = next(&mut cr).await;
    send(
        &mut cw,
        json!({"jsonrpc":"2.0","id":3,"method":"session/prompt",
        "params":{"prompt":[{"type":"text","text":"read src/a.rs"}]}}),
    )
    .await;

    // The agent calls back to the editor to read the file; answer it.
    let mut bridged = false;
    for _ in 0..20 {
        let m = next(&mut cr).await;
        if m.get("method").and_then(Value::as_str) == Some("fs/read_text_file") {
            assert_eq!(m["params"]["path"], "src/a.rs");
            bridged = true;
            send(
                &mut cw,
                json!({"jsonrpc":"2.0","id":m["id"].clone(),
                "result":{"content":"fn main() {}"}}),
            )
            .await;
        } else if m.get("id") == Some(&json!(3)) {
            break;
        }
    }
    assert!(
        bridged,
        "expected an fs/read_text_file request to the client"
    );

    drop(cw);
    task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_lifecycle_round_trips() {
    let dir = tmp("term");
    let config = config_at(&dir);
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "acp_terminal".into(),
            args: json!({"command": "ls"}),
        }],
        vec![Scripted::Text("ran it".into())],
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
        json!({"jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{"clientCapabilities":{"terminal":true}}}),
    )
    .await;
    let _ = next(&mut cr).await;
    send(
        &mut cw,
        json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}),
    )
    .await;
    let _ = next(&mut cr).await;
    send(
        &mut cw,
        json!({"jsonrpc":"2.0","id":3,"method":"session/prompt",
        "params":{"prompt":[{"type":"text","text":"run ls"}]}}),
    )
    .await;

    // Answer the terminal lifecycle methods the shim drives.
    let mut saw_create = false;
    let mut saw_output = false;
    for _ in 0..30 {
        let m = next(&mut cr).await;
        match m.get("method").and_then(Value::as_str) {
            Some("terminal/create") => {
                saw_create = true;
                send(
                    &mut cw,
                    json!({"jsonrpc":"2.0","id":m["id"].clone(),"result":{"terminalId":"t1"}}),
                )
                .await;
            }
            Some("terminal/output") => {
                saw_output = true;
                send(
                    &mut cw,
                    json!({"jsonrpc":"2.0","id":m["id"].clone(),
                    "result":{"output":"a.rs\nb.rs","exitCode":0}}),
                )
                .await;
            }
            Some("terminal/wait_for_exit") | Some("terminal/release") => {
                send(
                    &mut cw,
                    json!({"jsonrpc":"2.0","id":m["id"].clone(),"result":{}}),
                )
                .await;
            }
            _ => {
                if m.get("id") == Some(&json!(3)) {
                    break;
                }
            }
        }
    }
    assert!(saw_create, "expected terminal/create");
    assert!(saw_output, "expected terminal/output");

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

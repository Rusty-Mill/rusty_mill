#![cfg(feature = "mcp-server")]
//! `rusty-keys --mcp` acceptance: a client talks JSON-RPC to the `chat` tool,
//! and each call runs a full `Session::send` turn (real registry, workspace
//! policy, verifier, evidence journal) against a scripted fake model. Offline.
//! The server is `rusty_mcp_server` driven over in-memory pipes; its wire
//! behaviour against an independent client is tested in that crate.

use std::io::{self, Cursor, Write};
use std::sync::{Arc, Mutex};

use rk_app::mcp_server::build_server;
use rk_app::Session;
use rk_config::Config;
use rk_kernel::fake::{FakeLanguageModel, Scripted};
use serde_json::{json, Value};

#[derive(Clone, Default)]
struct Out(Arc<Mutex<Vec<u8>>>);

impl Write for Out {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn workspace(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rk-mcp-server-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn config_at(ws: &std::path::Path) -> Config {
    let ws = ws.to_string_lossy().into_owned();
    Config::resolve(move |k| match k {
        "RUSTYKEYS_MODEL" => Some("fake".into()),
        "RUSTYKEYS_WORKSPACE" => Some(ws.clone()),
        _ => None,
    })
    .unwrap()
}

/// Send `requests` (one JSON object per line) to a server over `model` and
/// return every reply, in the order written. `after_session` runs once the
/// session exists, before any request, for tests that break something.
async fn exchange_with(
    ws: &std::path::Path,
    model: FakeLanguageModel,
    requests: &[Value],
    after_session: impl FnOnce(),
) -> Vec<Value> {
    let session = Session::new(&config_at(ws), model).unwrap();
    after_session();
    let server = build_server(Arc::new(session), tokio::runtime::Handle::current()).unwrap();
    let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
    let out = Out::default();
    let sink = out.clone();
    tokio::task::spawn_blocking(move || {
        rusty_mcp_server::serve_lines(
            Arc::new(server),
            Cursor::new(input.into_bytes()),
            sink,
            rusty_mcp_server::StdioConfig::default(),
        )
    })
    .await
    .unwrap()
    .unwrap();
    let bytes = out.0.lock().unwrap().clone();
    String::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

async fn exchange(
    ws: &std::path::Path,
    model: FakeLanguageModel,
    requests: &[Value],
) -> Vec<Value> {
    exchange_with(ws, model, requests, || {}).await
}

fn reply(out: &[Value], id: i64) -> &Value {
    out.iter()
        .find(|m| m["id"] == id)
        .unwrap_or_else(|| panic!("no reply to {id}: {out:?}"))
}

fn init() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"ide","version":"1"}}})
}

fn chat(id: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{
        "name":"chat","arguments":{"message": message}}})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_identifies_itself_and_offers_the_chat_tool() {
    let ws = workspace("list");
    let model = FakeLanguageModel::new(vec![]);
    let list = json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
    let out = exchange(&ws, model, &[init(), list]).await;

    let info = &reply(&out, 1)["result"];
    assert_eq!(info["serverInfo"]["name"], "rusty-keys");
    assert_eq!(info["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        info["instructions"],
        "Rusty Keys harness exposed over MCP. Call `chat` to run a turn."
    );
    assert!(info["capabilities"]["tools"].is_object());

    let tools = reply(&out, 2)["result"]["tools"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "chat");
    assert_eq!(
        tools[0]["description"],
        "Send a message to Rusty Keys and receive a reply."
    );
    assert_eq!(tools[0]["inputSchema"]["required"], json!(["message"]));
    assert_eq!(
        tools[0]["inputSchema"]["properties"]["session_id"]["type"],
        "string"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_chat_call_runs_a_full_verified_turn() {
    let ws = workspace("turn");
    std::fs::write(ws.join("note.txt"), "hello from the workspace").unwrap();
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "read_file".into(),
            args: json!({"path": "note.txt"}),
        }],
        vec![Scripted::Text("I read the note.".into())],
    ]);
    let out = exchange(&ws, model, &[init(), chat(2, "what is in note.txt?")]).await;

    let result = &reply(&out, 2)["result"];
    assert_eq!(result["isError"], false);
    assert_eq!(result["content"][0]["type"], "text");
    assert_eq!(result["content"][0]["text"], "I read the note.");

    // The same evidence a CLI turn leaves: the turn was journaled and verified.
    let journal = std::fs::read_to_string(ws.join(".rustykeys/evidence.jsonl")).unwrap();
    assert!(journal.contains("\"kind\":\"turn\""), "{journal}");
    assert!(journal.contains("\"verified\":true"), "{journal}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_chat_call_goes_through_the_one_session() {
    let ws = workspace("two");
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::Text("first".into())],
        vec![Scripted::Text("second".into())],
    ]);
    // Both calls are admitted at once and share the session, so which gets
    // which scripted reply is up to scheduling; each reply is used once.
    let out = exchange(&ws, model, &[init(), chat(2, "one"), chat(3, "two")]).await;
    let mut replies: Vec<&str> = [2, 3]
        .iter()
        .map(|id| {
            reply(&out, *id)["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
        })
        .collect();
    replies.sort_unstable();
    assert_eq!(replies, ["first", "second"]);
    // And both turns are in the same evidence journal.
    let journal = std::fs::read_to_string(ws.join(".rustykeys/evidence.jsonl")).unwrap();
    assert_eq!(journal.matches("\"kind\":\"turn\"").count(), 2, "{journal}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_turn_is_a_tool_error_not_a_protocol_error() {
    let ws = workspace("fail");
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("never delivered".into())]]);
    // Make the evidence journal unwritable once the session exists, so the
    // turn itself fails after the model has answered.
    let broken = ws.clone();
    let out = exchange_with(&ws, model, &[init(), chat(2, "hello")], move || {
        let dir = broken.join(".rustykeys");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::write(&dir, "not a directory").unwrap();
    })
    .await;
    let m = reply(&out, 2);
    assert!(
        m.get("error").is_none(),
        "a harness failure is a result: {m}"
    );
    assert_eq!(m["result"]["isError"], true, "{m}");
    assert!(
        m["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("harness error: "),
        "{m}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_tool_is_invalid_params_and_a_stateless_client_works() {
    let ws = workspace("stateless");
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("hi".into())]]);
    let meta = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28"});
    let unknown = json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"nope","_meta": meta}});
    let list = json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta": meta}});
    let call = json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
        "params":{"name":"chat","arguments":{"message":"hi"},"_meta": meta}});
    let out = exchange(&ws, model, &[unknown, list, call]).await;

    // The old `rmcp` server answered -32601 here; the spec's code for an
    // unknown tool is -32602.
    assert_eq!(reply(&out, 1)["error"]["code"], -32602);
    // 2026-07-28 lists carry cache hints, as `apply_cache_hints` set them before.
    assert_eq!(reply(&out, 2)["result"]["ttlMs"], 0);
    assert_eq!(reply(&out, 2)["result"]["cacheScope"], "public");
    assert_eq!(reply(&out, 3)["result"]["content"][0]["text"], "hi");
    assert_eq!(reply(&out, 3)["result"]["resultType"], "complete");
}

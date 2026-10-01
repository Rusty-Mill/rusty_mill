//! MCP conformance suite for `adk-mcp` (design review Tranche 4).
//!
//! Wire-level only: every case goes through a public transport (the stdio
//! stream, the HTTP router, or `McpToolset` over a real subprocess), never
//! through the crate's internals, so the same suite verifies any
//! implementation behind that surface. It pins the MCP `2025-06-18`
//! behavior that other-language ADK clients rely on, plus this crate's own
//! hardening (the 16 MiB line cap and the client request deadline).
//!
//! `harness = false`: the binary is also the MCP server the client cases
//! launch. With `ADK_MCP_CONFORMANCE_ROLE=server` it serves the test tools
//! over stdio; with `=stall` it reads stdin and never answers.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use adk_core::{InvocationContext, RunConfig, Schema, SchemaType, Services, Session};
use adk_mcp::{router, serve_stream, BoundMcpToolset, ConnectionParams, McpServer, McpToolset};
use adk_sessions::InMemorySessionService;
use adk_tools::{FunctionTool, SharedTool, ToolContext, Toolset};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const ROLE: &str = "ADK_MCP_CONFORMANCE_ROLE";
const VERSION: &str = "2025-06-18";
const WAIT: Duration = Duration::from_secs(10);

type Outcome = Result<(), String>;
type Case = fn() -> Pin<Box<dyn Future<Output = Outcome>>>;

macro_rules! ensure {
    ($cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err(format!($($msg)+));
        }
    };
}

/// Set when the confirmation-gated tool's body runs, which it never may.
static APPROVE_ME_RAN: AtomicBool = AtomicBool::new(false);

/// One case per function, named after it.
macro_rules! cases {
    ($($name:ident),* $(,)?) => {
        &[$((stringify!($name), || Box::pin($name()) as Pin<Box<dyn Future<Output = Outcome>>>)),*]
    };
}

const CASES: &[(&str, Case)] = cases![
    stdio_initialize,
    stdio_notification,
    stdio_tools_list,
    stdio_call_ok,
    stdio_call_tool_error,
    stdio_call_unknown,
    stdio_call_bad_args,
    stdio_call_gated,
    stdio_unknown_method,
    stdio_malformed,
    stdio_oversized,
    http_round_trip,
    client_discovers,
    client_filter,
    client_calls,
    client_stall,
    client_reconnects,
];

fn main() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    match std::env::var(ROLE).as_deref() {
        Ok("server") => runtime.block_on(serve_role()),
        Ok("stall") => runtime.block_on(stall_role()),
        _ => {
            let args = Args::parse(std::env::args().skip(1));
            if args.list {
                args.selected()
                    .for_each(|(name, _)| println!("{name}: test"));
                return;
            }
            std::process::exit(runtime.block_on(run_cases(&args)));
        }
    }
}

/// The slice of libtest's command line that `cargo test` and nextest use:
/// `--list` (with `--format terse`), a name filter, `--exact`, and
/// `--ignored` (no case here is ignored, so it selects none). Anything
/// else (`--nocapture`, `--test-threads`, ...) is accepted and ignored.
struct Args {
    list: bool,
    exact: bool,
    ignored_only: bool,
    filter: Option<String>,
}

impl Args {
    fn parse(args: impl Iterator<Item = String>) -> Self {
        let mut parsed = Self {
            list: false,
            exact: false,
            ignored_only: false,
            filter: None,
        };
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--list" => parsed.list = true,
                "--exact" => parsed.exact = true,
                "--ignored" => parsed.ignored_only = true,
                // Flags that take a value: skip the value too.
                "--format" | "--test-threads" | "--color" | "--logfile" | "--skip" => {
                    args.next();
                }
                flag if flag.starts_with('-') => {}
                name => parsed.filter = Some(name.to_string()),
            }
        }
        parsed
    }

    fn selected(&self) -> impl Iterator<Item = &'static (&'static str, Case)> + '_ {
        CASES.iter().filter(move |(name, _)| {
            !self.ignored_only
                && self.filter.as_deref().is_none_or(|filter| {
                    if self.exact {
                        *name == filter
                    } else {
                        name.contains(filter)
                    }
                })
        })
    }
}

async fn serve_role() {
    if let Err(err) = adk_mcp::serve_stdio(&test_server()).await {
        eprintln!("conformance server failed: {err}");
        std::process::exit(1);
    }
}

async fn stall_role() {
    let _ = tokio::io::copy(&mut tokio::io::stdin(), &mut tokio::io::sink()).await;
}

async fn run_cases(args: &Args) -> i32 {
    let (mut passed, mut failed) = (0, 0);
    for (name, case) in args.selected() {
        match timeout(Duration::from_secs(60), case()).await {
            Ok(Ok(())) => {
                passed += 1;
                println!("ok      {name}");
            }
            Ok(Err(why)) => {
                failed += 1;
                println!("FAILED  {name}: {why}");
            }
            Err(_) => {
                failed += 1;
                println!("FAILED  {name}: did not finish within 60s");
            }
        }
    }
    println!("\n{passed} passed; {failed} failed");
    i32::from(failed > 0)
}

// ---------------------------------------------------------------- fixtures

fn test_server() -> McpServer {
    let echo = FunctionTool::new(
        "echo",
        "Echoes its text.",
        Schema::object().property("text", Schema::string()),
        |args, _ctx| {
            Box::pin(async move {
                let text = args.get("text").cloned().unwrap_or(Value::Null);
                Ok(adk_tools::success(json!({ "text": text })))
            })
        },
    );
    let fail = FunctionTool::new("fail", "Always fails.", Schema::object(), |_a, _c| {
        Box::pin(async { Err(adk_core::AdkError::tool("fail", "exploded")) })
    });
    let approve_me = FunctionTool::new("approve_me", "Gated.", Schema::object(), |_a, _c| {
        Box::pin(async {
            APPROVE_ME_RAN.store(true, Ordering::SeqCst);
            Ok(adk_tools::success(json!({ "ran": true })))
        })
    })
    .require_confirmation("Approve?");
    let tools: Vec<SharedTool> = vec![echo.shared(), fail.shared(), approve_me.shared()];
    McpServer::new("conformance", tools, services())
}

fn services() -> Services {
    Services::new(Arc::new(InMemorySessionService::new()))
}

fn invocation() -> InvocationContext {
    let session = Session::new("conformance-session", "conformance", "user");
    InvocationContext::new(session, services(), RunConfig::default())
}

fn initialize_params() -> Value {
    json!({
        "protocolVersion": VERSION,
        "capabilities": {},
        "clientInfo": {"name": "conformance", "version": "0"},
    })
}

/// A JSON-RPC peer speaking newline-delimited JSON to `serve_stream`.
struct StdioPeer {
    to_server: DuplexStream,
    from_server: BufReader<DuplexStream>,
    server: JoinHandle<adk_core::Result<()>>,
}

impl StdioPeer {
    fn start() -> Self {
        let (to_server, server_in) = tokio::io::duplex(1 << 16);
        let (server_out, from_server) = tokio::io::duplex(1 << 16);
        let server = Arc::new(test_server());
        let server =
            tokio::spawn(async move { serve_stream(&server, server_in, server_out).await });
        Self {
            to_server,
            from_server: BufReader::new(from_server),
            server,
        }
    }

    /// A peer that has completed the `initialize` handshake.
    async fn initialized() -> Result<Self, String> {
        let mut peer = Self::start();
        peer.request(0, "initialize", initialize_params()).await?;
        peer.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await?;
        Ok(peer)
    }

    async fn send(&mut self, message: Value) -> Outcome {
        self.send_raw(&format!("{message}\n")).await
    }

    async fn send_raw(&mut self, text: &str) -> Outcome {
        self.to_server
            .write_all(text.as_bytes())
            .await
            .map_err(|e| format!("write failed: {e}"))
    }

    async fn next(&mut self) -> Result<Value, String> {
        let mut line = String::new();
        let read = timeout(WAIT, self.from_server.read_line(&mut line))
            .await
            .map_err(|_| "no message from the server within 10s".to_string())?
            .map_err(|e| format!("read failed: {e}"))?;
        ensure!(read > 0, "the server closed the stream");
        serde_json::from_str(&line).map_err(|e| format!("not JSON ({e}): {line}"))
    }

    /// Sends a request and returns its response, skipping notifications.
    async fn request(&mut self, id: i64, method: &str, params: Value) -> Result<Value, String> {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        loop {
            let message = self.next().await?;
            if message.get("id") == Some(&json!(id)) {
                return Ok(message);
            }
        }
    }

    async fn call(&mut self, id: i64, params: Value) -> Result<Value, String> {
        self.request(id, "tools/call", params).await
    }
}

fn result_of(response: &Value) -> Result<&Value, String> {
    ensure!(
        response.get("error").is_none(),
        "unexpected error: {response}"
    );
    response
        .get("result")
        .ok_or_else(|| format!("no result: {response}"))
}

fn is_error_response(response: &Value) -> bool {
    response.get("error").is_some() && response.get("result").is_none()
}

/// The JSON a tool result carries in its first text block.
fn text_payload(result: &Value) -> Result<Value, String> {
    ensure!(
        result["content"][0]["type"] == "text",
        "not a text block: {result}"
    );
    let text = result["content"][0]["text"]
        .as_str()
        .ok_or_else(|| format!("no text: {result}"))?;
    serde_json::from_str(text).map_err(|e| format!("text is not JSON ({e}): {text}"))
}

// ------------------------------------------------------------- stdio server

async fn stdio_initialize() -> Outcome {
    let mut peer = StdioPeer::start();
    let response = peer.request(1, "initialize", initialize_params()).await?;
    let result = result_of(&response)?;
    ensure!(result["protocolVersion"] == VERSION, "version: {result}");
    ensure!(
        result["serverInfo"]["name"] == "conformance",
        "serverInfo: {result}"
    );
    ensure!(
        result["capabilities"]["tools"].is_object(),
        "capabilities: {result}"
    );
    Ok(())
}

async fn stdio_notification() -> Outcome {
    let mut peer = StdioPeer::initialized().await?;
    peer.send(json!({"jsonrpc": "2.0", "id": 7, "method": "ping"}))
        .await?;
    let first = peer.next().await?;
    ensure!(
        first["id"] == 7,
        "expected the ping reply first, got {first}"
    );
    ensure!(first["result"].is_object(), "ping result: {first}");
    Ok(())
}

async fn stdio_tools_list() -> Outcome {
    let mut peer = StdioPeer::initialized().await?;
    let response = peer.request(1, "tools/list", json!({})).await?;
    let tools = result_of(&response)?["tools"]
        .as_array()
        .ok_or_else(|| format!("no tools array: {response}"))?;
    let echo = tools
        .iter()
        .find(|t| t["name"] == "echo")
        .ok_or_else(|| format!("echo missing: {response}"))?;
    ensure!(
        echo["description"] == "Echoes its text.",
        "description: {echo}"
    );
    ensure!(echo["inputSchema"]["type"] == "object", "schema: {echo}");
    ensure!(
        echo["inputSchema"]["properties"]["text"]["type"] == "string",
        "property type: {echo}"
    );
    ensure!(tools.iter().any(|t| t["name"] == "fail"), "fail missing");
    Ok(())
}

async fn stdio_call_ok() -> Outcome {
    let mut peer = StdioPeer::initialized().await?;
    let response = peer
        .call(1, json!({"name": "echo", "arguments": {"text": "hi"}}))
        .await?;
    let result = result_of(&response)?;
    ensure!(result["isError"] == false, "isError: {result}");
    let payload = text_payload(result)?;
    ensure!(payload["status"] == "success", "status: {payload}");
    ensure!(payload["text"] == "hi", "echo: {payload}");
    Ok(())
}

async fn stdio_call_tool_error() -> Outcome {
    let mut peer = StdioPeer::initialized().await?;
    let response = peer
        .call(1, json!({"name": "fail", "arguments": {}}))
        .await?;
    let result = result_of(&response)?;
    ensure!(result["isError"] == true, "isError: {result}");
    ensure!(
        text_payload(result)?["status"] == "error",
        "status: {result}"
    );
    Ok(())
}

async fn stdio_call_unknown() -> Outcome {
    let mut peer = StdioPeer::initialized().await?;
    let response = peer
        .call(1, json!({"name": "nope", "arguments": {}}))
        .await?;
    ensure!(
        is_error_response(&response),
        "expected an error: {response}"
    );
    Ok(())
}

async fn stdio_call_bad_args() -> Outcome {
    let mut peer = StdioPeer::initialized().await?;
    let response = peer
        .call(1, json!({"name": "echo", "arguments": [1]}))
        .await?;
    ensure!(
        is_error_response(&response),
        "expected an error: {response}"
    );
    Ok(())
}

async fn stdio_call_gated() -> Outcome {
    let mut peer = StdioPeer::initialized().await?;
    let response = peer
        .call(1, json!({"name": "approve_me", "arguments": {}}))
        .await?;
    ensure!(
        is_error_response(&response),
        "expected an error: {response}"
    );
    let message = response["error"]["message"].as_str().unwrap_or_default();
    ensure!(message.contains("confirmation"), "message: {message}");
    ensure!(!APPROVE_ME_RAN.load(Ordering::SeqCst), "the gated tool ran");
    Ok(())
}

async fn stdio_unknown_method() -> Outcome {
    let mut peer = StdioPeer::initialized().await?;
    let response = peer.request(1, "resources/nonexistent", json!({})).await?;
    ensure!(response["error"]["code"] == -32601, "code: {response}");
    Ok(())
}

async fn stdio_malformed() -> Outcome {
    let mut peer = StdioPeer::initialized().await?;
    peer.send_raw("{not json\n").await?;
    let response = peer.request(9, "ping", json!({})).await?;
    result_of(&response)?;
    Ok(())
}

async fn stdio_oversized() -> Outcome {
    let StdioPeer {
        mut to_server,
        from_server,
        server,
    } = StdioPeer::start();
    // Keep draining the server's output so it can never block on a write.
    let drain = tokio::spawn(async move {
        let mut from_server = from_server;
        let _ = tokio::io::copy(&mut from_server, &mut tokio::io::sink()).await;
    });
    let chunk = vec![b'a'; 1 << 20];
    for _ in 0..17 {
        if to_server.write_all(&chunk).await.is_err() {
            break; // the server already stopped reading
        }
    }
    let ended = timeout(WAIT, server)
        .await
        .map_err(|_| "the server kept reading past the cap".to_string())?
        .map_err(|e| format!("server task panicked: {e}"))?;
    drain.abort();
    ensure!(ended.is_err(), "expected serve_stream to fail at the cap");
    Ok(())
}

// -------------------------------------------------------------- HTTP server

struct HttpReply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl HttpReply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The JSON-RPC message with `id`, from a JSON or an SSE body.
    fn message(&self, id: i64) -> Result<Value, String> {
        let event_stream = self
            .header("content-type")
            .is_some_and(|t| t.starts_with("text/event-stream"));
        let candidates: Vec<&str> = if event_stream {
            self.body
                .lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .map(str::trim)
                .collect()
        } else {
            vec![self.body.as_str()]
        };
        candidates
            .into_iter()
            .filter_map(|text| serde_json::from_str::<Value>(text).ok())
            .find(|m| m.get("id") == Some(&json!(id)))
            .ok_or_else(|| format!("no message with id {id} in: {}", self.body))
    }
}

async fn post(port: u16, session: Option<&str>, body: &Value) -> Result<HttpReply, String> {
    let body = body.to_string();
    let mut request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nMCP-Protocol-Version: {VERSION}\r\n\
         Content-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(session) = session {
        request.push_str(&format!("Mcp-Session-Id: {session}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(&body);

    let exchange = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
        stream.write_all(request.as_bytes()).await?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await?;
        Ok::<_, std::io::Error>(raw)
    };
    let raw = timeout(WAIT, exchange)
        .await
        .map_err(|_| "no HTTP reply within 10s".to_string())?
        .map_err(|e| format!("HTTP exchange failed: {e}"))?;
    parse_http(&raw)
}

fn parse_http(raw: &[u8]) -> Result<HttpReply, String> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| format!("no header end: {text}"))?;
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("bad status line: {head}"))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let chunked = headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"));
    let body = if chunked {
        dechunk(body)
    } else {
        body.to_string()
    };
    Ok(HttpReply {
        status,
        headers,
        body,
    })
}

fn dechunk(mut body: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = body.split_once("\r\n") {
        let Ok(size) = usize::from_str_radix(size.trim(), 16) else {
            break;
        };
        if size == 0 || rest.len() < size {
            break;
        }
        out.push_str(&rest[..size]);
        body = rest[size..].trim_start_matches("\r\n");
    }
    out
}

async fn http_round_trip() -> Outcome {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("bind: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let app = router(Arc::new(test_server()), "/mcp");
    let server = tokio::spawn(async move { axum::serve(listener, app).await });

    let init =
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": initialize_params()});
    let reply = post(port, None, &init).await?;
    ensure!(
        reply.status == 200,
        "initialize status {}: {}",
        reply.status,
        reply.body
    );
    let session = reply.header("mcp-session-id").map(str::to_string);
    let response = reply.message(1)?;
    ensure!(
        result_of(&response)?["protocolVersion"] == VERSION,
        "version: {response}"
    );

    let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    let reply = post(port, session.as_deref(), &note).await?;
    ensure!(reply.status == 202, "notification status {}", reply.status);

    let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}});
    let response = post(port, session.as_deref(), &list).await?.message(2)?;
    let names: Vec<&Value> = result_of(&response)?["tools"]
        .as_array()
        .map(|tools| tools.iter().map(|t| &t["name"]).collect())
        .unwrap_or_default();
    ensure!(names.contains(&&json!("echo")), "tools: {response}");

    let call = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                      "params": {"name": "echo", "arguments": {"text": "over http"}}});
    let response = post(port, session.as_deref(), &call).await?.message(3)?;
    ensure!(
        text_payload(result_of(&response)?)?["text"] == "over http",
        "call: {response}"
    );

    server.abort();
    Ok(())
}

// ------------------------------------------------------------------- client

fn server_params(role: &str) -> Result<ConnectionParams, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    Ok(ConnectionParams::stdio(exe.to_string_lossy(), Vec::<String>::new()).with_env(ROLE, role))
}

async fn bound_tools(toolset: McpToolset) -> Result<(BoundMcpToolset, Vec<SharedTool>), String> {
    let bound = BoundMcpToolset::new(toolset);
    let tools = bound
        .tools(&invocation())
        .await
        .map_err(|e| format!("tools(): {e}"))?;
    Ok((bound, tools))
}

fn find<'a>(tools: &'a [SharedTool], name: &str) -> Result<&'a SharedTool, String> {
    tools
        .iter()
        .find(|t| t.name() == name)
        .ok_or_else(|| format!("{name} not discovered"))
}

async fn client_discovers() -> Outcome {
    let (bound, tools) = bound_tools(McpToolset::new(server_params("server")?)).await?;
    let echo = find(&tools, "echo")?;
    ensure!(echo.description() == "Echoes its text.", "description");
    let schema = echo
        .declaration()
        .and_then(|d| d.parameters)
        .ok_or("echo has no parameters")?;
    ensure!(
        schema.schema_type == Some(SchemaType::Object),
        "schema type"
    );
    ensure!(
        schema.properties["text"].schema_type == Some(SchemaType::String),
        "property type"
    );
    bound.close().await.map_err(|e| e.to_string())
}

async fn client_filter() -> Outcome {
    let toolset = McpToolset::new(server_params("server")?).with_filter(["echo"]);
    let (bound, tools) = bound_tools(toolset).await?;
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    ensure!(names == ["echo"], "filtered tools: {names:?}");
    bound.close().await.map_err(|e| e.to_string())
}

async fn client_calls() -> Outcome {
    let (bound, tools) = bound_tools(McpToolset::new(server_params("server")?)).await?;
    let ctx = ToolContext::new(invocation());

    let mut args = serde_json::Map::new();
    args.insert("text".into(), json!("from rust"));
    let ok = find(&tools, "echo")?
        .run(args, &ctx)
        .await
        .map_err(|e| format!("echo: {e}"))?;
    ensure!(ok["status"] == "success", "echo status: {ok}");
    ensure!(ok["text"] == "from rust", "echo text: {ok}");

    let failed = find(&tools, "fail")?
        .run(serde_json::Map::new(), &ctx)
        .await
        .map_err(|e| format!("fail: {e}"))?;
    ensure!(failed["status"] == "error", "fail status: {failed}");
    ensure!(
        failed["error_message"]
            .as_str()
            .is_some_and(|m| m.contains("exploded")),
        "fail message: {failed}"
    );
    bound.close().await.map_err(|e| e.to_string())
}

async fn client_stall() -> Outcome {
    let toolset = McpToolset::new(server_params("stall")?).with_timeout(Duration::from_millis(300));
    let bound = BoundMcpToolset::new(toolset);
    let started = std::time::Instant::now();
    let result = timeout(WAIT, bound.tools(&invocation()))
        .await
        .map_err(|_| "tools() hung on a stalled server".to_string())?;
    ensure!(result.is_err(), "expected an error from a stalled server");
    ensure!(
        started.elapsed() < Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );
    bound.close().await.map_err(|e| e.to_string())
}

async fn client_reconnects() -> Outcome {
    let (bound, tools) = bound_tools(McpToolset::new(server_params("server")?)).await?;
    bound.close().await.map_err(|e| format!("close: {e}"))?;
    let mut args = serde_json::Map::new();
    args.insert("text".into(), json!("again"));
    let ctx = ToolContext::new(invocation());
    let reply = find(&tools, "echo")?
        .run(args, &ctx)
        .await
        .map_err(|e| format!("call after close: {e}"))?;
    ensure!(reply["text"] == "again", "reply: {reply}");
    bound.close().await.map_err(|e| e.to_string())
}

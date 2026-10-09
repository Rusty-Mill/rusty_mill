//! Protocol-only MCP client for stdio and Streamable HTTP servers, on
//! `rusty_mcp_client_native`.
//!
//! [`McpServerSpec`] describes the connection and [`McpTransport`] selects
//! its transport. Host applications own configuration files, policy checks,
//! connection pooling, and tool dispatch.
//!
//! [`McpClient::connect`] resolves authentication and shakes hands (it asks
//! for `server/discover` first and falls back to `initialize` for a classic
//! server). [`McpClient::shutdown`] closes the connection without blocking the
//! caller. WebSocket is a reserved schema value and returns an explicit
//! unsupported-transport error.
//!
//! The values it returns are `rusty_mcp_proto`'s (re-exported as
//! [`crate::proto`]); they used to be `rmcp`'s.

use std::io;
use std::time::Duration;

use rusty_mcp_client_native::proto::{CallToolResult, Message, Prompt, Resource, Tool};
use rusty_mcp_client_native::{
    AsyncClient, ClientConfig, ClientError, HttpConfig, HttpTransport, NoHandler, Recv,
    StdioTransport, Transport,
};

use crate::client_auth::{self as auth, AuthError, McpAuth};

use serde::{Deserialize, Serialize};

/// Wire transport used to connect to an MCP server.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    /// Spawn a child process and communicate over standard input/output.
    #[default]
    Stdio,
    /// Connect to a Streamable HTTP endpoint.
    Http,
    /// Reserved for compatibility; MCP WebSocket transport is unsupported.
    Websocket,
}

/// Protocol-only configuration for one MCP client connection.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct McpServerSpec {
    /// Transport to use.
    #[serde(default)]
    pub transport: McpTransport,
    /// Child executable for stdio.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    /// Child arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Child environment.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    /// Streamable HTTP endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Legacy static bearer value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,
    /// HTTP headers sent on every request.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub headers: std::collections::BTreeMap<String, String>,
    /// Declarative HTTP authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<McpAuth>,
    /// Policy hint for adapters; the protocol client does not interpret it.
    #[serde(default)]
    pub disabled: bool,
}

/// P2-06 — default timeout for the MCP initialize handshake. A
/// non-responding server binary would otherwise hang connect for
/// however long its stdout takes to unblock. Override via a future
/// `[mcp.timeouts] connect_secs = N` block (deferred from P2-06).
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = DEFAULT_CONNECT_TIMEOUT;

/// P2-06 — nominal budget for graceful shutdown, kept as public API. The
/// transport gives a stdio child 2 s to exit once its stdin is closed, then
/// kills it, and bounds the HTTP session `DELETE` at 2 s.
pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors from the MCP Host client.
#[derive(Debug, thiserror::Error)]
pub enum McpClientError {
    /// Failed to spawn the server process (command not on PATH, permission
    /// denied, etc.).
    #[error("spawn {command}: {source}")]
    Spawn {
        /// Command that failed to spawn.
        command: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The MCP initialize handshake failed or timed out.
    #[error("initialize handshake failed: {reason}")]
    Handshake {
        /// Why the handshake failed.
        reason: String,
    },

    /// Configuration error — the spec is missing required fields for its
    /// declared transport (e.g. `transport = "http"` with no `url`).
    #[error("transport config: {reason}")]
    Config {
        /// Why the spec was rejected before any I/O happened.
        reason: String,
    },

    /// The transport listed in the spec is recognised but not currently
    /// dispatchable in this build (e.g. `transport = "websocket"`, which no
    /// MCP client here implements).
    #[error("transport unsupported: {reason}")]
    Unsupported {
        /// Human-readable explanation including a migration hint.
        reason: String,
    },

    /// BL-025 — auth resolution failed before the transport could be
    /// constructed (missing env var, OAuth token endpoint refused,
    /// malformed token response, …).
    #[error("auth: {0}")]
    Auth(#[from] AuthError),

    /// Any runtime error from the underlying client (transport closed,
    /// protocol violation, etc.).
    #[error("mcp service error: {0}")]
    Service(String),
}

impl McpClientError {
    /// Whether the error represents a transient runtime failure that may
    /// succeed on retry. `Service` errors are transient (transport blip,
    /// remote restart). `Spawn` and `Handshake` are not — both indicate
    /// misconfiguration that retrying would just delay surfacing.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Service(_))
    }
}

/// The two ways to reach a server, behind one [`Transport`].
enum Link {
    Stdio(StdioTransport),
    Http(Box<HttpTransport>),
}

impl Transport for Link {
    fn send(&mut self, message: &Message) -> io::Result<()> {
        match self {
            Self::Stdio(t) => t.send(message),
            Self::Http(t) => t.send(message),
        }
    }

    fn recv(&mut self, timeout: Duration) -> io::Result<Recv> {
        match self {
            Self::Stdio(t) => t.recv(timeout),
            Self::Http(t) => t.recv(timeout),
        }
    }

    fn set_protocol_version(&mut self, version: &rusty_mcp_client_native::proto::ProtocolVersion) {
        match self {
            Self::Stdio(t) => t.set_protocol_version(version),
            Self::Http(t) => t.set_protocol_version(version),
        }
    }
}

/// A live connection to one external MCP server.
///
/// Calls from several tasks queue and run one at a time, in order. Dropping
/// the client closes the transport (and stops a stdio child); [`shutdown`]
/// does the same without blocking the caller.
///
/// [`shutdown`]: McpClient::shutdown
pub struct McpClient {
    /// Human-readable name (the key from `mcp.toml`), used in logs and error
    /// messages.
    name: String,
    client: AsyncClient<Link, NoHandler>,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient")
            .field("name", &self.name)
            .field("protocol", &self.client.negotiated())
            .finish_non_exhaustive()
    }
}

fn service(e: ClientError) -> McpClientError {
    McpClientError::Service(e.to_string())
}

/// RFC 9110 token characters: what a header name may be made of.
fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// Visible ASCII, space and tab: no CR, LF, NUL or other control bytes.
fn valid_header_value(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b == b'\t' || (0x20..=0x7e).contains(&b))
}

impl McpClient {
    /// Spawn the configured external MCP server (or reach its URL) and shake
    /// hands. Returns once the server has reported its capabilities.
    ///
    /// The child's stderr is inherited (not captured) so the operator sees
    /// server-side startup logs in their terminal — critical for diagnosing
    /// misconfigured server binaries.
    ///
    /// # Errors
    /// - [`McpClientError::Spawn`] if the executable cannot be started.
    /// - [`McpClientError::Handshake`] if the handshake fails or a step
    ///   exceeds [`DEFAULT_CONNECT_TIMEOUT`].
    pub async fn connect(name: &str, spec: &McpServerSpec) -> Result<Self, McpClientError> {
        match spec.transport {
            McpTransport::Stdio => Self::connect_stdio(name, spec).await,
            McpTransport::Http => Self::connect_http(name, spec).await,
            McpTransport::Websocket => Err(McpClientError::Unsupported {
                reason: format!(
                    "server '{name}': WebSocket transport is reserved in the config schema \
                     but not implemented (the MCP 2025-03-26 spec deprecates WebSocket in \
                     favour of `transport = \"http\"`). \
                     Switch to `transport = \"http\"` to connect to this server."
                ),
            }),
        }
    }

    /// Stdio path — spawn the configured command and run the MCP handshake
    /// over the child's stdio.
    async fn connect_stdio(name: &str, spec: &McpServerSpec) -> Result<Self, McpClientError> {
        if spec.command.trim().is_empty() {
            return Err(McpClientError::Config {
                reason: format!("server '{name}': stdio transport needs a non-empty `command`"),
            });
        }
        let mut command = std::process::Command::new(&spec.command);
        command.args(&spec.args).envs(&spec.env);
        let transport = StdioTransport::spawn(command).map_err(|e| McpClientError::Spawn {
            command: spec.command.clone(),
            source: e,
        })?;
        Self::run_handshake(name, Link::Stdio(transport)).await
    }

    /// Streamable HTTP path — the modern remote MCP transport (single
    /// endpoint, POST for requests + SSE for the server stream). Auth
    /// headers and custom HTTP headers from the spec are forwarded on
    /// every request. Redirects are not followed: they would carry those
    /// headers to wherever the redirect points.
    async fn connect_http(name: &str, spec: &McpServerSpec) -> Result<Self, McpClientError> {
        let url = spec.url.as_deref().unwrap_or("").trim();
        if url.is_empty() {
            return Err(McpClientError::Config {
                reason: format!("server '{name}': http transport needs `url = \"https://…\"`"),
            });
        }

        // BL-025 — resolve the optional `auth` declaration up front so any
        // missing env-var or OAuth endpoint failure surfaces with a clear
        // `Auth` error before we construct the transport. A static
        // `auth_header` from the file still works (back-compat with
        // pre-BL-025 installs) but a present `auth` block always wins on
        // conflict — declarative beats legacy.
        let resolved_auth = if let Some(auth_decl) = spec.auth.as_ref() {
            Some(auth::resolve(auth_decl).await?)
        } else {
            None
        };

        // Validate custom headers up front so a malformed entry surfaces as a
        // clean Config error rather than a failure on the first request.
        let extra = resolved_auth.iter().flat_map(|r| r.extra_headers.iter());
        let mut headers: Vec<(String, String)> = Vec::new();
        for (k, v) in spec.headers.iter().chain(extra) {
            if !valid_header_name(k) {
                return Err(McpClientError::Config {
                    reason: format!("server '{name}': invalid header name '{k}'"),
                });
            }
            if !valid_header_value(v) {
                return Err(McpClientError::Config {
                    reason: format!("server '{name}': invalid header value for '{k}'"),
                });
            }
            headers.push((k.clone(), v.clone()));
        }

        // Resolved auth wins; otherwise fall back to the static
        // `auth_header` field (the BL-023 path).
        let authorization = resolved_auth
            .as_ref()
            .and_then(|r| r.authorization.clone())
            .or_else(|| spec.auth_header.clone());
        let mut config = HttpConfig::new(url);
        config.headers = headers;
        // Nexus accepts both `Bearer token` and bare `token` forms. Remove
        // exactly one case-insensitive Bearer prefix before the transport
        // supplies the scheme, so we never send `Bearer Bearer token`.
        config.bearer_token = authorization.map(|auth| {
            auth.split_once(' ')
                .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
                .map_or(auth.as_str(), |(_, token)| token)
                .to_owned()
        });
        let transport = HttpTransport::new(config).map_err(|e| McpClientError::Config {
            reason: format!("server '{name}': {e}"),
        })?;
        Self::run_handshake(name, Link::Http(Box::new(transport))).await
    }

    /// Shared handshake driver: every step is bounded by
    /// [`DEFAULT_CONNECT_TIMEOUT`], so a non-responsive server doesn't hang
    /// `connect` forever. Later calls are bounded by the client's call timeout
    /// instead (ten minutes unless the config says otherwise).
    async fn run_handshake(name: &str, link: Link) -> Result<Self, McpClientError> {
        let config = ClientConfig::new("rusty-mcp-client", env!("CARGO_PKG_VERSION"));
        let client = AsyncClient::connect(link, config, NoHandler, CONNECT_TIMEOUT)
            .await
            .map_err(|e| McpClientError::Handshake {
                reason: e.to_string(),
            })?;
        Ok(Self {
            name: name.to_string(),
            client,
        })
    }

    /// Logical name of this connection (the key from `mcp.toml`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Fetch every tool the server exposes, transparently paginating until
    /// the server reports no further cursor.
    ///
    /// # Errors
    /// [`McpClientError::Service`] on transport failure or protocol error.
    pub async fn list_tools(&self) -> Result<Vec<Tool>, McpClientError> {
        self.client.list_tools().await.map_err(service)
    }

    /// Fetch every resource the server exposes.
    ///
    /// # Errors
    /// [`McpClientError::Service`] on transport failure or protocol error.
    pub async fn list_resources(&self) -> Result<Vec<Resource>, McpClientError> {
        self.client.list_resources().await.map_err(service)
    }

    /// Fetch every prompt template the server exposes.
    ///
    /// # Errors
    /// [`McpClientError::Service`] on transport failure or protocol error.
    pub async fn list_prompts(&self) -> Result<Vec<Prompt>, McpClientError> {
        self.client.list_prompts().await.map_err(service)
    }

    /// Invoke a tool by name with the given JSON arguments.
    ///
    /// `arguments` is an optional `serde_json::Map` matching the tool's
    /// declared input schema. Pass `None` for tools that take no arguments.
    /// A tool that *fails* comes back as `Ok` with `is_error` set, as the
    /// protocol defines; a server that asks the user a question mid-call is
    /// answered "declined".
    ///
    /// # Errors
    /// [`McpClientError::Service`] on transport failure, a protocol error, or
    /// an error the server reports for the call itself.
    pub async fn call_tool(
        &self,
        name: impl Into<String>,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, McpClientError> {
        let arguments = arguments
            .map(|map| {
                let text = serde_json::Value::Object(map).to_string();
                rusty_mcp_client_native::json::Value::from_json_str(&text)
            })
            .transpose()
            .map_err(|e| McpClientError::Service(format!("arguments: {e}")))?;
        self.client
            .call_tool(&name.into(), arguments)
            .await
            .map_err(service)
    }

    /// Close the connection: the call in flight (if any) finishes, the
    /// transport is closed (a stdio child is stopped, an HTTP session
    /// ended), and the caller is not blocked meanwhile.
    ///
    /// # Errors
    /// [`McpClientError::Service`] if the close could not run.
    pub async fn shutdown(self) -> Result<(), McpClientError> {
        self.client.close().await.map_err(service)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    const TEST_API_KEY: &str = "synthetic-api-key";
    const TEST_BEARER_TOKEN: &str = "synthetic-bearer";

    async fn read_http_request(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 1024];
            let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut chunk))
                .await
                .expect("request read timed out")
                .expect("request read failed");
            assert!(read > 0, "connection closed before request completed");
            request.extend_from_slice(&chunk[..read]);
            if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                break end + 4;
            }
            assert!(request.len() <= 64 * 1024, "test request headers too large");
        };

        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("valid content length"))
            })
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1024];
            let read = stream
                .read(&mut chunk)
                .await
                .expect("request body read failed");
            assert!(read > 0, "connection closed before request body completed");
            request.extend_from_slice(&chunk[..read]);
        }
        String::from_utf8(request).expect("synthetic request must be UTF-8")
    }

    async fn write_http_response(stream: &mut TcpStream, status: &str, headers: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("response write failed");
    }

    fn http_spec(url: String, auth_header: &str) -> McpServerSpec {
        let mut headers = std::collections::BTreeMap::new();
        headers.insert("X-Nexus-Test-Key".to_string(), TEST_API_KEY.to_string());
        McpServerSpec {
            transport: McpTransport::Http,
            url: Some(url),
            auth_header: Some(auth_header.to_string()),
            headers,
            ..McpServerSpec::default()
        }
    }

    /// The only thing we can meaningfully unit-test at this layer is the
    /// spawn-failure path, because connecting to a real MCP server requires
    /// a whole other binary. A "server binary does not exist on PATH" check
    /// exercises the spawn surface end-to-end without needing one.
    #[tokio::test]
    async fn connect_fails_for_nonexistent_command() {
        let spec = McpServerSpec {
            command: "this-binary-definitely-does-not-exist-12345".to_string(),
            ..McpServerSpec::default()
        };
        let err = McpClient::connect("test", &spec)
            .await
            .expect_err("invalid connection specification must fail");
        assert!(
            matches!(err, McpClientError::Spawn { .. }),
            "expected Spawn error, got {err:?}"
        );
    }

    /// Handshake must time out if the spawned process never writes to
    /// stdout. We use `/usr/bin/yes` as a "never-respond-to-MCP" stand-in:
    /// it writes to stdout indefinitely but never produces valid MCP
    /// framing, so the initialize read stalls and the timeout fires.
    ///
    /// Skipped on platforms where `yes` is not available; this is a
    /// best-effort smoke test rather than a portability guarantee.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn connect_times_out_when_server_never_speaks_mcp() {
        // `sleep 60` is portable and quieter than `yes`: it writes nothing,
        // so the transport's framed reader blocks forever. Our 15 s
        // CONNECT_TIMEOUT would be too long for CI, so we override by
        // shortening via direct tokio timeout around the future. But the
        // public API caps at CONNECT_TIMEOUT, so we simply skip this test
        // if the binary is missing and otherwise tolerate the wait.
        if std::process::Command::new("sleep")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let spec = McpServerSpec {
            command: "sleep".to_string(),
            args: vec!["60".to_string()],
            ..McpServerSpec::default()
        };
        // Directly bound the connect call below CONNECT_TIMEOUT so CI
        // completes quickly. The production timeout still applies in real
        // usage; here we only want to prove the spawn + transport path
        // doesn't panic when the child refuses to speak MCP.
        let connect = McpClient::connect("test", &spec);
        let short = tokio::time::timeout(Duration::from_millis(500), connect).await;
        // Expect either the local short timeout fired (Err) OR the rmcp
        // side errored out fast. Either way, we must not get `Ok`.
        match short {
            Err(_elapsed) => {}
            Ok(Err(_)) => {}
            Ok(Ok(_)) => panic!("connect should not have succeeded against a non-MCP process"),
        }
    }

    // ── BL-023 — transport dispatch ──────────────────────────────────────

    #[tokio::test]
    async fn websocket_transport_returns_unsupported() {
        // The transport variant is reserved (see McpTransport doc); the
        // connect-time dispatch must surface a clear error pointing at
        // the http alternative rather than silently falling back.
        let spec = McpServerSpec {
            transport: McpTransport::Websocket,
            url: Some("wss://example.com/mcp".into()),
            ..McpServerSpec::default()
        };
        let err = McpClient::connect("legacy", &spec)
            .await
            .expect_err("invalid connection specification must fail");
        match err {
            McpClientError::Unsupported { reason } => {
                assert!(
                    reason.contains("WebSocket"),
                    "error should mention WebSocket: {reason}"
                );
                assert!(
                    reason.contains("http"),
                    "error should suggest http alternative: {reason}"
                );
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn http_transport_rejects_missing_url_with_config_error() {
        let spec = McpServerSpec {
            transport: McpTransport::Http,
            url: None,
            ..McpServerSpec::default()
        };
        let err = McpClient::connect("remote", &spec)
            .await
            .expect_err("invalid connection specification must fail");
        assert!(
            matches!(err, McpClientError::Config { .. }),
            "expected Config, got {err:?}"
        );
    }

    #[tokio::test]
    async fn http_transport_rejects_invalid_header_name() {
        // Headers are validated up-front so malformed entries don't reach
        // the wire (the rmcp transport would otherwise lazily error on
        // first request, which is harder to diagnose).
        let mut headers = std::collections::BTreeMap::new();
        headers.insert("not a valid header".to_string(), "value".to_string());
        let spec = McpServerSpec {
            transport: McpTransport::Http,
            url: Some("https://example.com/mcp".into()),
            headers,
            ..McpServerSpec::default()
        };
        let err = McpClient::connect("remote", &spec)
            .await
            .expect_err("invalid connection specification must fail");
        assert!(
            matches!(err, McpClientError::Config { .. }),
            "expected Config, got {err:?}"
        );
    }

    #[tokio::test]
    async fn http_transport_delivers_custom_and_bearer_headers_directly() {
        for auth_header in [TEST_BEARER_TOKEN, "bEaReR synthetic-bearer"] {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind synthetic server");
            let url = format!(
                "http://{}/mcp",
                listener.local_addr().expect("bound server address")
            );
            let server = tokio::spawn(async move {
                // The client asks `server/discover` first; this classic
                // server does not know it, which sends the client on to
                // `initialize`. Every request must carry the headers.
                let check = |request: &str| {
                    let lower = request.to_ascii_lowercase();
                    assert!(
                        lower.contains("x-nexus-test-key: synthetic-api-key"),
                        "custom header missing from direct request: {request}"
                    );
                    assert!(
                        lower.contains("authorization: bearer synthetic-bearer"),
                        "bearer header missing from direct request: {request}"
                    );
                };
                let id_of = |request: &str| {
                    serde_json::from_str::<serde_json::Value>(
                        request
                            .split_once("\r\n\r\n")
                            .expect("HTTP header terminator")
                            .1,
                    )
                    .expect("valid request JSON")["id"]
                        .clone()
                };

                let (mut discover, _) = listener.accept().await.expect("accept client request");
                let request = read_http_request(&mut discover).await;
                check(&request);
                assert!(request.contains("server/discover"), "{request}");
                let body = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id_of(&request),
                    "error": { "code": -32601, "message": "method not found" }
                })
                .to_string();
                write_http_response(
                    &mut discover,
                    "200 OK",
                    "Content-Type: application/json\r\n",
                    &body,
                )
                .await;

                let (mut initialize, _) = listener.accept().await.expect("accept client request");
                let request = read_http_request(&mut initialize).await;
                check(&request);
                assert!(request.contains("\"initialize\""), "{request}");
                let body = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id_of(&request),
                    "result": {
                        "protocolVersion": "2025-03-26",
                        "capabilities": {},
                        "serverInfo": { "name": "synthetic", "version": "1.0.0" }
                    }
                })
                .to_string();
                write_http_response(
                    &mut initialize,
                    "200 OK",
                    "Content-Type: application/json\r\n",
                    &body,
                )
                .await;

                let (mut initialized, _) = listener.accept().await.expect("accept client request");
                let request = read_http_request(&mut initialized).await;
                check(&request);
                assert!(request.contains("notifications/initialized"));
                write_http_response(&mut initialized, "202 Accepted", "", "").await;
            });

            let client = McpClient::connect("direct", &http_spec(url, auth_header))
                .await
                .expect("synthetic MCP handshake should succeed");
            client.shutdown().await.expect("shutdown should succeed");
            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .expect("synthetic server did not finish")
                .expect("synthetic server panicked");
        }
    }

    #[tokio::test]
    async fn http_transport_does_not_forward_custom_headers_across_redirects() {
        for status in ["307 Temporary Redirect", "308 Permanent Redirect"] {
            let target = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind synthetic server");
            let redirect = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind synthetic server");
            let target_url = format!(
                "http://{}/capture",
                target.local_addr().expect("bound server address")
            );
            let url = format!(
                "http://{}/mcp",
                redirect.local_addr().expect("bound server address")
            );

            let redirect_server = tokio::spawn(async move {
                let (mut stream, _) = redirect.accept().await.expect("accept client request");
                let request = read_http_request(&mut stream).await;
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("x-nexus-test-key: synthetic-api-key")
                );
                write_http_response(
                    &mut stream,
                    status,
                    &format!("Location: {target_url}\r\n"),
                    "",
                )
                .await;
            });

            let result = McpClient::connect(
                "redirect",
                &http_spec(url, &format!("Bearer {TEST_BEARER_TOKEN}")),
            )
            .await;
            assert!(
                matches!(result, Err(McpClientError::Handshake { .. })),
                "redirect must fail closed instead of completing a handshake: {result:?}"
            );
            tokio::time::timeout(Duration::from_secs(2), redirect_server)
                .await
                .expect("redirect origin was not contacted")
                .expect("redirect origin panicked");
            assert!(
                tokio::time::timeout(Duration::from_millis(300), target.accept())
                    .await
                    .is_err(),
                "{status} target was contacted; synthetic credentials may have leaked"
            );
        }
    }

    #[tokio::test]
    async fn http_transport_handshake_times_out_against_dead_endpoint() {
        // 127.0.0.1 with a deliberately-unused port. We don't wait for
        // the production CONNECT_TIMEOUT (15s) — bound the test future
        // tighter via a local timeout. The point is to prove the connect
        // surface routes through `connect_http` without panicking.
        let spec = McpServerSpec {
            transport: McpTransport::Http,
            // Port 1 is reserved; OS rejects the TCP connect immediately.
            url: Some("http://127.0.0.1:1/mcp".into()),
            ..McpServerSpec::default()
        };
        let connect = McpClient::connect("dead", &spec);
        let short = tokio::time::timeout(Duration::from_millis(2_000), connect).await;
        match short {
            Err(_elapsed) => {}
            Ok(Err(_)) => {} // surfaced as Handshake / transport error
            Ok(Ok(_)) => panic!("connect should not have succeeded against 127.0.0.1:1"),
        }
    }
}

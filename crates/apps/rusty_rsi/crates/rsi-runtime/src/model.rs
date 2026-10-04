//! [`ChatModel`] adapters (ADR-0005 §8).
//!
//! - [`OpenAiModel`]: any OpenAI-compatible `/chat/completions` endpoint
//!   over plain HTTP, such as a local Ollama. Token usage comes from the
//!   response's `usage` field; a response without it is an error, because
//!   an unmetered call would defeat the budget.
//!
//!   A call never outlives its deadline. The endpoint's address is
//!   resolved once, when the client is built, so no budgeted call waits on
//!   DNS; connecting and every read and write share one absolute deadline.
//!   With an API key configured, nothing the endpoint sent reaches an error
//!   message, since an endpoint may echo the key it rejected.
//! - [`ScriptedModel`]: canned replies with fixed token costs, for tests and
//!   CI, which have no model.

use std::cell::Cell;
use std::fmt::Display;
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rsi_core::{ChatModel, Completion, Message, ModelId};
use rusty_http::body::{response_framing, Framing};
use rusty_http::head::RequestHead;
use rusty_http::sync::SyncTransport;
use rusty_http::{HeaderMap, Method, Url, Version};
use rusty_json::Value;

use crate::error::RuntimeError;

/// The largest response head accepted.
const MAX_HEAD_BYTES: usize = 64 * 1024;
/// The largest response body accepted.
const MAX_BODY_BYTES: u64 = 16 << 20;
/// How long building a client may wait for a host name lookup.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);

/// A host name lookup: `(host, port)` to addresses.
type Resolve = fn(&str, u16) -> std::io::Result<Vec<SocketAddr>>;

fn system_resolve(host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
    (host, port).to_socket_addrs().map(Iterator::collect)
}

/// Whether a lookup is running on a resolver thread. std's resolver
/// cannot be cancelled, so a lookup that outlives its waiter keeps running;
/// allowing only one at a time means stalled lookups never pile up.
static LOOKUP_RUNNING: AtomicBool = AtomicBool::new(false);

/// An OpenAI-compatible chat endpoint over plain HTTP. Each call ends by
/// the earlier of its own timeout and the caller's.
pub struct OpenAiModel {
    id: ModelId,
    endpoint: Url,
    address: SocketAddr,
    api_key: Option<String>,
    timeout: Duration,
}

impl std::fmt::Debug for OpenAiModel {
    // Hand-written so the API key can never reach a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiModel")
            .field("id", &self.id)
            .field("endpoint", &self.endpoint.absolute_form())
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl OpenAiModel {
    /// A client for model `id` at `base_url` (for example
    /// `http://127.0.0.1:11434/v1`), sending `api_key` as a bearer token
    /// when set, and giving up on a call after `timeout`. A host name is
    /// resolved here, once, waiting at most 10 seconds.
    ///
    /// # Errors
    /// [`RuntimeError::Model`] for an unparsable URL, an `https` one (TLS
    /// is not wired in yet, so a remote endpoint needs a local proxy), or a
    /// host name that does not resolve in time.
    pub fn new(
        id: ModelId,
        base_url: &str,
        api_key: Option<String>,
        timeout: Duration,
    ) -> Result<Self, RuntimeError> {
        Self::with_resolver(
            id,
            base_url,
            api_key,
            timeout,
            system_resolve,
            RESOLVE_TIMEOUT,
        )
    }

    fn with_resolver(
        id: ModelId,
        base_url: &str,
        api_key: Option<String>,
        timeout: Duration,
        resolve: Resolve,
        resolve_timeout: Duration,
    ) -> Result<Self, RuntimeError> {
        let mut endpoint = Url::parse(base_url)
            .map_err(|e| RuntimeError::Model(format!("endpoint {base_url}: {e}")))?;
        if endpoint.scheme != "http" {
            return Err(RuntimeError::Model(format!(
                "endpoint {base_url}: only http:// is supported"
            )));
        }
        endpoint.path = format!("{}/chat/completions", endpoint.path.trim_end_matches('/'));
        let address = resolve_once(&endpoint.host, endpoint.port, resolve, resolve_timeout)
            .map_err(|e| RuntimeError::Model(format!("resolving {}: {e}", endpoint.host)))?;
        Ok(Self {
            id,
            endpoint,
            address,
            api_key,
            timeout,
        })
    }

    /// `what: detail` for an error, or just `what` when an API key is
    /// configured: `detail` came from the endpoint and may echo the key.
    fn endpoint_error(&self, what: &str, detail: impl Display) -> RuntimeError {
        let detail = if self.api_key.is_some() {
            "details withheld: an API key is configured".to_owned()
        } else {
            detail.to_string()
        };
        RuntimeError::Model(format!("{what} ({}): {detail}", self.endpoint.host))
    }

    /// Posts `body` and returns the response body, all within `timeout`.
    fn post(&self, body: &[u8], timeout: Duration) -> Result<Vec<u8>, RuntimeError> {
        let fail = |what: String| RuntimeError::Model(format!("{what} ({})", self.endpoint.host));
        let timeout = timeout.min(self.timeout);
        if timeout.is_zero() {
            return Err(fail("no time left for the call".into()));
        }
        let deadline = Instant::now() + timeout;
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(fail("no time left to connect".into()));
        }
        let stream = TcpStream::connect_timeout(&self.address, left)
            .map_err(|e| fail(format!("connecting: {e}")))?;
        let mut headers = HeaderMap::new();
        let header = |headers: &mut HeaderMap, name: &str, value: &str| {
            headers
                .insert(name, value)
                .map_err(|e| fail(format!("header {name}: {e}")))
        };
        header(&mut headers, "Host", &self.endpoint.host_header())?;
        header(&mut headers, "Content-Type", "application/json")?;
        header(&mut headers, "Content-Length", &body.len().to_string())?;
        header(&mut headers, "Connection", "close")?;
        if let Some(key) = &self.api_key {
            headers
                .insert_sensitive("Authorization", &format!("Bearer {key}"))
                .map_err(|_| fail("the API key is not a valid header value".into()))?;
        }
        let head = RequestHead {
            method: Method::Post,
            target: self.endpoint.request_target(),
            version: Version::Http11,
            headers,
        };
        let mut transport = SyncTransport::new(Deadline { stream, deadline });
        transport
            .write_request_head(&head)
            .and_then(|()| transport.write_body(body))
            .map_err(|e| fail(format!("sending: {e}")))?;
        // From here on, parser errors quote what the endpoint sent.
        let response = transport
            .read_response_head(MAX_HEAD_BYTES)
            .map_err(|e| self.endpoint_error("reading the response head", e))?;
        let framing = response_framing(&response.headers, &Method::Post, response.status)
            .map_err(|e| self.endpoint_error("response framing", e))?;
        let body = match read_capped_body(transport, framing, MAX_BODY_BYTES) {
            Ok(body) => body,
            Err(BodyError::TooLarge) => {
                return Err(fail(format!(
                    "the response body exceeds {MAX_BODY_BYTES} bytes"
                )));
            }
            Err(BodyError::Read(e)) => return Err(self.endpoint_error("reading the body", e)),
        };
        if !response.status.is_success() {
            let status = response.status.as_u16();
            // An endpoint may echo the credential it rejected, in full or in
            // part, so with a key configured the body never reaches an error.
            if self.api_key.is_some() {
                return Err(fail(format!(
                    "HTTP {status} (response body withheld: an API key is configured)"
                )));
            }
            let text = String::from_utf8_lossy(&body);
            let preview: String = text.chars().take(300).collect();
            return Err(fail(format!("HTTP {status}: {preview}")));
        }
        Ok(body)
    }
}

/// Why a body could not be read.
#[derive(Debug)]
enum BodyError {
    /// It passed the size limit.
    TooLarge,
    /// The transport or the framing failed; the error may quote the body.
    Read(rusty_http::TransportError),
}

/// Reads a response body in any framing, failing as soon as it passes
/// `limit` bytes rather than buffering it first.
fn read_capped_body<T: Read>(
    transport: SyncTransport<T>,
    framing: Framing,
    limit: u64,
) -> Result<Vec<u8>, BodyError> {
    if let Framing::ContentLength(len) = framing {
        if len > limit {
            return Err(BodyError::TooLarge);
        }
    }
    let mut reader = transport.into_body_reader(framing);
    let mut body = Vec::new();
    while let Some(chunk) = reader.next_chunk().map_err(BodyError::Read)? {
        if (body.len() + chunk.len()) as u64 > limit {
            return Err(BodyError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Resolves `host` once: an IP literal directly, a name on a resolver
/// thread that may run for at most `timeout` before the caller gives up.
/// Only one lookup runs at a time ([`LOOKUP_RUNNING`]), so a resolver
/// that never returns strands one thread, not one per attempt.
fn resolve_once(
    host: &str,
    port: u16,
    resolve: Resolve,
    timeout: Duration,
) -> std::io::Result<SocketAddr> {
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = literal.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    if LOOKUP_RUNNING.swap(true, Ordering::AcqRel) {
        return Err(std::io::Error::other(
            "an earlier lookup has not finished; refusing to start another",
        ));
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let name = host.to_owned();
    std::thread::spawn(move || {
        let result = resolve(&name, port);
        LOOKUP_RUNNING.store(false, Ordering::Release);
        // The caller may have given up; nobody is left to tell.
        let _ = sender.send(result);
    });
    let addresses = receiver
        .recv_timeout(timeout)
        .map_err(|_| std::io::Error::new(ErrorKind::TimedOut, "the lookup timed out"))??;
    addresses
        .into_iter()
        .next()
        .ok_or_else(|| std::io::Error::other("no address"))
}

/// A TCP stream whose every read and write must finish by `deadline`: the
/// whole call is bounded, not just each read, so a server that drips one
/// byte at a time cannot stretch it.
struct Deadline {
    stream: TcpStream,
    deadline: Instant,
}

impl Deadline {
    /// Runs `op` with the socket timeouts set to the time left, retrying
    /// spurious timeouts until the deadline has really passed.
    fn within<R>(
        &mut self,
        mut op: impl FnMut(&mut TcpStream) -> std::io::Result<R>,
    ) -> std::io::Result<R> {
        loop {
            let left = self.deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(std::io::Error::new(
                    ErrorKind::TimedOut,
                    "the call's deadline passed",
                ));
            }
            self.stream.set_read_timeout(Some(left))?;
            self.stream.set_write_timeout(Some(left))?;
            match op(&mut self.stream) {
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                result => return result,
            }
        }
    }
}

impl Read for Deadline {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.within(|stream| stream.read(buf))
    }
}

impl Write for Deadline {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.within(|stream| stream.write(buf))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.within(Write::flush)
    }
}

/// The JSON body of a chat-completion request.
fn request_body(model: &ModelId, messages: &[Message], max_tokens: u64) -> String {
    let mut list = Value::array();
    for message in messages {
        let mut item = Value::object();
        item.insert("role", message.role.as_str());
        item.insert("content", message.content.as_str());
        list.push(item);
    }
    let mut body = Value::object();
    body.insert("model", model.as_str());
    body.insert("messages", list);
    body.insert("max_tokens", max_tokens);
    body.insert("stream", false);
    body.to_json_string()
}

/// Why a response body is not a usable completion.
#[derive(Debug)]
enum ParseError {
    /// Not JSON; the parser's message may quote the body.
    Json(String),
    /// JSON of the wrong shape; the message is ours.
    Shape(String),
}

/// The reply text and token usage of a chat-completion response.
fn parse_completion(body: &[u8]) -> Result<Completion, ParseError> {
    let bad = |what: &str| ParseError::Shape(what.to_owned());
    let text = std::str::from_utf8(body).map_err(|_| bad("not UTF-8"))?;
    let json = Value::parse(text).map_err(|e| ParseError::Json(e.to_string()))?;
    let reply = json
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("no choices[0].message.content"))?;
    let tokens = |key: &str| {
        json.pointer(&format!("/usage/{key}"))
            .and_then(Value::as_u64)
            .ok_or_else(|| bad(&format!("no usage.{key}; refusing an unmetered call")))
    };
    Ok(Completion {
        text: reply.to_owned(),
        prompt_tokens: tokens("prompt_tokens")?,
        completion_tokens: tokens("completion_tokens")?,
    })
}

impl ChatModel for OpenAiModel {
    type Error = RuntimeError;

    fn id(&self) -> &ModelId {
        &self.id
    }

    fn complete(
        &self,
        messages: &[Message],
        max_tokens: u64,
        timeout: Duration,
    ) -> Result<Completion, RuntimeError> {
        let body = request_body(&self.id, messages, max_tokens);
        parse_completion(&self.post(body.as_bytes(), timeout)?).map_err(|e| match e {
            ParseError::Json(detail) => self.endpoint_error("the response is not JSON", detail),
            ParseError::Shape(what) => RuntimeError::Model(format!("unexpected response: {what}")),
        })
    }
}

/// Canned replies, cycled in order, each charged a fixed token cost.
#[derive(Debug)]
pub struct ScriptedModel {
    id: ModelId,
    replies: Vec<String>,
    prompt_tokens: u64,
    completion_tokens: u64,
    calls: Cell<usize>,
}

impl ScriptedModel {
    /// A model that answers with `replies` in turn, charging
    /// `prompt_tokens` plus `completion_tokens` (capped at the call's
    /// `max_tokens`) per call.
    #[must_use]
    pub const fn new(
        id: ModelId,
        replies: Vec<String>,
        prompt_tokens: u64,
        completion_tokens: u64,
    ) -> Self {
        Self {
            id,
            replies,
            prompt_tokens,
            completion_tokens,
            calls: Cell::new(0),
        }
    }

    /// Calls answered so far.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.get()
    }
}

impl ChatModel for ScriptedModel {
    type Error = RuntimeError;

    fn id(&self) -> &ModelId {
        &self.id
    }

    fn complete(
        &self,
        _: &[Message],
        max_tokens: u64,
        _: Duration,
    ) -> Result<Completion, RuntimeError> {
        let call = self.calls.get();
        let text = self
            .replies
            .get(call % self.replies.len().max(1))
            .ok_or_else(|| RuntimeError::Model("the script has no replies".into()))?;
        self.calls.set(call + 1);
        Ok(Completion {
            text: text.clone(),
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens.min(max_tokens),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;

    use rsi_core::Role;

    use super::*;

    fn id() -> ModelId {
        ModelId::parse("qwen2.5-coder:7b").expect("valid")
    }

    fn messages() -> Vec<Message> {
        vec![
            Message {
                role: Role::System,
                content: "be \"brief\"".into(),
            },
            Message {
                role: Role::User,
                content: "hi\n".into(),
            },
        ]
    }

    #[test]
    fn builds_an_openai_request_body() {
        let body = Value::parse(&request_body(&id(), &messages(), 77)).expect("JSON");
        assert_eq!(
            body.pointer("/model").and_then(Value::as_str),
            Some("qwen2.5-coder:7b")
        );
        assert_eq!(
            body.pointer("/messages/0/content").and_then(Value::as_str),
            Some("be \"brief\"")
        );
        assert_eq!(
            body.pointer("/messages/1/role").and_then(Value::as_str),
            Some("user")
        );
        assert_eq!(
            body.pointer("/max_tokens").and_then(Value::as_u64),
            Some(77)
        );
    }

    #[test]
    fn parses_completions_and_refuses_unmetered_ones() {
        let ok = br#"{"choices":[{"message":{"role":"assistant","content":"hey"}}],
                     "usage":{"prompt_tokens":12,"completion_tokens":3}}"#;
        assert_eq!(
            parse_completion(ok).ok(),
            Some(Completion {
                text: "hey".into(),
                prompt_tokens: 12,
                completion_tokens: 3
            })
        );
        let unmetered = br#"{"choices":[{"message":{"content":"hey"}}]}"#;
        assert!(parse_completion(unmetered).is_err());
        assert!(parse_completion(b"{}").is_err());
        assert!(parse_completion(b"not json").is_err());
    }

    #[test]
    fn refuses_https_and_redacts_the_key() {
        let https = OpenAiModel::new(id(), "https://api.example/v1", None, Duration::from_secs(1));
        assert!(matches!(https, Err(RuntimeError::Model(_))));
        let model = OpenAiModel::new(
            id(),
            "http://127.0.0.1:1/v1/",
            Some("sk-secret".into()),
            Duration::from_secs(1),
        )
        .expect("valid");
        let debug = format!("{model:?}");
        assert!(!debug.contains("sk-secret"), "{debug}");
        assert!(debug.contains("/v1/chat/completions"), "{debug}");
    }

    const SECOND: Duration = Duration::from_secs(1);

    /// How a test endpoint answers.
    type Respond = dyn FnOnce(&mut TcpStream) + Send;

    /// A loopback server playing the endpoint for one request: it reads the
    /// request, hands the connection to `respond`, and returns the request's
    /// head and body.
    fn endpoint(
        respond: impl FnOnce(&mut TcpStream) + Send + 'static,
    ) -> (String, std::thread::JoinHandle<(String, String)>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream);
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("line");
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().expect("length");
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).expect("body");
            respond(reader.get_mut());
            (head, String::from_utf8(body).expect("utf8"))
        });
        (format!("http://127.0.0.1:{port}/v1"), server)
    }

    fn client(url: &str, key: Option<&str>) -> OpenAiModel {
        OpenAiModel::new(id(), url, key.map(str::to_owned), Duration::from_secs(600))
            .expect("valid")
    }

    #[test]
    fn talks_to_an_openai_compatible_server() {
        let (url, server) = endpoint(|stream| {
            let reply = r#"{"choices":[{"message":{"content":"pong"}}],"usage":{"prompt_tokens":5,"completion_tokens":1}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{reply}",
                reply.len()
            )
            .expect("reply");
        });
        let model = client(&url, Some("sk-test"));
        let completion = model
            .complete(&messages(), 9, 5 * SECOND)
            .expect("completes");
        assert_eq!(completion.text, "pong");
        assert_eq!(completion.prompt_tokens + completion.completion_tokens, 6);
        let (head, body) = server.join().expect("server");
        assert!(
            head.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"),
            "{head}"
        );
        assert!(head.contains("Authorization: Bearer sk-test"), "{head}");
        assert!(body.contains("\"max_tokens\":9"), "{body}");
    }

    #[test]
    fn a_silent_or_dripping_endpoint_cannot_outlast_the_timeout() {
        for drip in [false, true] {
            let (url, server) = endpoint(move |stream| {
                // Either say nothing, or one byte every 100 ms: each read
                // succeeds, so only a deadline on the whole call stops it.
                for _ in 0..50 {
                    if drip && stream.write_all(b"H").is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
            let started = Instant::now();
            let result = client(&url, None).complete(&messages(), 1, Duration::from_millis(500));
            let elapsed = started.elapsed();
            assert!(matches!(result, Err(RuntimeError::Model(_))), "{result:?}");
            assert!(elapsed >= Duration::from_millis(500), "{elapsed:?}");
            assert!(elapsed < 2 * SECOND, "drip {drip}: {elapsed:?}");
            server.join().expect("server");
        }
    }

    #[test]
    fn oversized_responses_are_refused_in_every_framing() {
        let big = MAX_BODY_BYTES as usize + 1;
        let chunked = move |stream: &mut TcpStream| {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
            let chunk = vec![b'a'; 1 << 20];
            let mut sent = 0;
            while sent < big {
                let header = format!("{:x}\r\n", chunk.len());
                let wrote = stream
                    .write_all(header.as_bytes())
                    .and_then(|()| stream.write_all(&chunk))
                    .and_then(|()| stream.write_all(b"\r\n"));
                if wrote.is_err() {
                    return; // the client gave up, as it should
                }
                sent += chunk.len();
            }
            let _ = stream.write_all(b"0\r\n\r\n");
        };
        let declared = move |stream: &mut TcpStream| {
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {big}\r\n\r\n");
        };
        let closed = move |stream: &mut TcpStream| {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
            let _ = stream.write_all(&vec![b'a'; big]);
        };
        let framings: [(&str, Box<Respond>); 3] = [
            ("chunked", Box::new(chunked)),
            ("content-length", Box::new(declared)),
            ("close-delimited", Box::new(closed)),
        ];
        for (name, respond) in framings {
            let (url, server) = endpoint(respond);
            let result = client(&url, None).complete(&messages(), 1, 30 * SECOND);
            match result {
                Err(RuntimeError::Model(message)) => {
                    assert!(message.contains("exceeds"), "{name}: {message}");
                }
                other => panic!("{name}: {other:?}"),
            }
            server.join().expect("server");
        }
    }

    #[test]
    fn an_echoed_api_key_never_reaches_an_error() {
        const SENTINEL: &str = "sk-SENTINEL-0123456789abcdef";
        let (url, server) = endpoint(|stream| {
            let body =
                format!(r#"{{"error":"Incorrect API key provided: {SENTINEL} (sk-SENT...cdef)"}}"#);
            write!(
                stream,
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .expect("reply");
        });
        let error = client(&url, Some(SENTINEL))
            .complete(&messages(), 1, 5 * SECOND)
            .expect_err("401");
        server.join().expect("server");
        // Every way the error is shown: Display (the CLI prints
        // `rsi inner: {error}`), Debug, and the message the broker wraps.
        for shown in [
            error.to_string(),
            format!("{error:?}"),
            format!("rsi inner: {error}"),
        ] {
            assert!(!shown.contains("SENTINEL"), "{shown}");
            assert!(shown.contains("401"), "{shown}");
        }

        // Without a key there is nothing to leak, so the body is shown.
        let (url, server) = endpoint(|stream| {
            let body = "model not found";
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .expect("reply");
        });
        let error = client(&url, None)
            .complete(&messages(), 1, 5 * SECOND)
            .expect_err("404");
        server.join().expect("server");
        assert!(error.to_string().contains("model not found"), "{error}");
    }

    /// Every way an error is shown: `Display`, `Debug`, and the line the
    /// CLI prints (`rsi inner: {error}`).
    fn shown(error: &RuntimeError) -> [String; 3] {
        [
            error.to_string(),
            format!("{error:?}"),
            format!("rsi inner: {error}"),
        ]
    }

    #[test]
    fn endpoint_text_in_parser_errors_is_withheld_when_a_key_is_set() {
        const SENTINEL: &str = "sk-SENTINEL-0123456789abcdef";
        // Each response is malformed in a way whose parser error quotes it.
        let cases: [(&str, String); 4] = [
            (
                "response framing",
                format!("HTTP/1.1 200 OK\r\nContent-Length: {SENTINEL}\r\n\r\n"),
            ),
            (
                "reading the body",
                format!(
                    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{SENTINEL}\r\nx\r\n0\r\n\r\n"
                ),
            ),
            (
                "reading the response head",
                format!("HTTP/1.1 2{SENTINEL} OK\r\n\r\n"),
            ),
            (
                "not JSON",
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{SENTINEL}",
                    SENTINEL.len()
                ),
            ),
        ];
        for (category, response) in cases {
            for key in [Some(SENTINEL), None] {
                let reply = response.clone();
                let (url, server) = endpoint(move |stream| {
                    let _ = stream.write_all(reply.as_bytes());
                });
                let error = client(&url, key)
                    .complete(&messages(), 1, 5 * SECOND)
                    .expect_err("malformed");
                server.join().expect("server");
                for text in shown(&error) {
                    assert!(text.contains(category), "{category}: {text}");
                    if key.is_some() {
                        assert!(!text.contains("SENTINEL"), "{category}: {text}");
                    }
                }
                if key.is_none()
                    && matches!(category, "response framing" | "reading the response head")
                {
                    assert!(
                        error.to_string().contains("SENTINEL"),
                        "without a key the detail is kept: {error}"
                    );
                }
            }
        }
    }

    static NAMED_LOOKUPS: AtomicUsize = AtomicUsize::new(0);
    static HUNG_LOOKUPS: AtomicUsize = AtomicUsize::new(0);

    /// A resolver that takes 200 ms and finds loopback.
    fn slow_lookup(_: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
        NAMED_LOOKUPS.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(200));
        Ok(vec![SocketAddr::from(([127, 0, 0, 1], port))])
    }

    /// A resolver that stalls for 1.5 s and then fails.
    fn hung_lookup(_: &str, _: u16) -> std::io::Result<Vec<SocketAddr>> {
        HUNG_LOOKUPS.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(1500));
        Err(std::io::Error::other("stalled"))
    }

    fn never_lookup(_: &str, _: u16) -> std::io::Result<Vec<SocketAddr>> {
        panic!("an IP literal needs no lookup")
    }

    fn build(url: &str, resolve: Resolve, wait: Duration) -> Result<OpenAiModel, RuntimeError> {
        OpenAiModel::with_resolver(id(), url, None, 600 * SECOND, resolve, wait)
    }

    /// One test, because it drives the process-wide one-lookup guard.
    #[test]
    fn names_resolve_once_outside_any_call_and_stalled_lookups_do_not_pile_up() {
        assert!(build("http://127.0.0.1:9/v1", never_lookup, SECOND).is_ok());
        assert!(build("http://[::1]:9/v1", never_lookup, SECOND).is_ok());

        // A slow lookup happens once, at construction; calls never resolve.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        let model = build(&format!("http://model.test:{port}/v1"), slow_lookup, SECOND)
            .expect("resolves within its time");
        assert_eq!(model.address, SocketAddr::from(([127, 0, 0, 1], port)));
        for _ in 0..3 {
            let error = model
                .complete(&messages(), 1, Duration::from_millis(300))
                .expect_err("nothing listens");
            assert!(error.to_string().contains("connecting"), "{error}");
        }
        assert_eq!(
            NAMED_LOOKUPS.load(Ordering::SeqCst),
            1,
            "no lookup per call"
        );

        // A stalled lookup times out on schedule...
        let started = Instant::now();
        let stalled = build(
            "http://stalled.test:9/v1",
            hung_lookup,
            Duration::from_millis(300),
        );
        let waited = started.elapsed();
        assert!(stalled.is_err());
        assert!(
            waited >= Duration::from_millis(300) && waited < SECOND,
            "{waited:?}"
        );
        // ...and while it is still stuck, further attempts fail at once
        // instead of stranding another resolver thread each.
        for _ in 0..5 {
            let started = Instant::now();
            let refused = build("http://stalled.test:9/v1", hung_lookup, SECOND);
            assert!(
                refused
                    .as_ref()
                    .is_err_and(|e| e.to_string().contains("has not finished")),
                "{refused:?}"
            );
            assert!(started.elapsed() < Duration::from_millis(100));
        }
        assert_eq!(
            HUNG_LOOKUPS.load(Ordering::SeqCst),
            1,
            "one stranded lookup at most"
        );

        // Once the stalled lookup ends, lookups work again.
        std::thread::sleep(Duration::from_millis(1500));
        assert!(build("http://model.test:9/v1", slow_lookup, SECOND).is_ok());
    }

    #[test]
    fn an_unreachable_endpoint_is_a_model_error() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        let model = OpenAiModel::new(
            id(),
            &format!("http://127.0.0.1:{port}/v1"),
            None,
            Duration::from_secs(2),
        )
        .expect("valid");
        assert!(matches!(
            model.complete(&messages(), 1, 2 * SECOND),
            Err(RuntimeError::Model(_))
        ));
    }

    #[test]
    fn scripted_replies_cycle_and_respect_max_tokens() {
        let model = ScriptedModel::new(id(), vec!["a".into(), "b".into()], 10, 50);
        let texts: Vec<_> = (0..3)
            .map(|_| model.complete(&messages(), 20, SECOND).expect("reply"))
            .collect();
        assert_eq!(texts[0].text, "a");
        assert_eq!(texts[2].text, "a");
        assert_eq!(texts[1].completion_tokens, 20);
        assert_eq!(model.calls(), 3);
        let empty = ScriptedModel::new(id(), vec![], 1, 1);
        assert!(empty.complete(&messages(), 1, SECOND).is_err());
    }
}

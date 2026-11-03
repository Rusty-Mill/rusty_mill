//! One blocking chat completion: resolve, check, connect, send, read.

use std::io::{self, ErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use orch_dispatch::{AgentError, ClassifiedError};
use rusty_http::body::{self, DEFAULT_MAX_BODY_LEN};
use rusty_http::head::{RequestHead, ResponseHead, DEFAULT_MAX_HEAD_LEN};
use rusty_http::sync::SyncTransport;
use rusty_http::{HeaderMap, Method, StatusCode, TransportError, Version};

use crate::address::all_private;
use crate::wire::{parse_response, request_body, Completion, Prompt};
use crate::Config;

/// Interim (1xx) responses read past before giving up on a final one.
const MAX_INTERIM: usize = 8;
/// Lookups one client may have running at once, counting ones abandoned
/// at a deadline until they actually finish.
pub const MAX_PENDING_LOOKUPS: usize = 4;

/// Turns a host name into addresses. A seam so tests control DNS.
///
/// Lookups run on their own thread so the call's deadline bounds them; a
/// lookup still running at the deadline is abandoned, not cancelled.
pub trait Resolve: Send + Sync + 'static {
    fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>>;
}

/// The operating system's resolver.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        Ok((host, port).to_socket_addrs()?.collect())
    }
}

/// Reads one environment variable. A seam so tests never touch the process
/// environment.
pub type EnvLookup = fn(&str) -> Option<String>;

fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// A client for one OpenAI-compatible server. Holds no connection: each
/// [`OpenAiHttp::complete`] resolves, checks and connects afresh.
pub struct OpenAiHttp<R = SystemResolver> {
    config: Config,
    resolver: Arc<R>,
    env: EnvLookup,
    /// Lookup threads still running, including abandoned ones.
    pending: Arc<AtomicUsize>,
}

impl OpenAiHttp<SystemResolver> {
    /// A client over the system resolver and the process environment.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            resolver: Arc::new(SystemResolver),
            env: process_env,
            pending: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl<R: Resolve> OpenAiHttp<R> {
    /// Replace the resolver.
    pub fn with_resolver<S: Resolve>(self, resolver: S) -> OpenAiHttp<S> {
        OpenAiHttp {
            config: self.config,
            resolver: Arc::new(resolver),
            env: self.env,
            pending: self.pending,
        }
    }

    /// Replace the environment lookup used for the API key.
    pub fn with_env(self, env: EnvLookup) -> Self {
        Self { env, ..self }
    }

    /// The settings this client was built with.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Ask once. Errors are classified for fallback: a down or unreachable
    /// server, or a missing key, is `Unavailable`; a timeout, a rate limit
    /// or a server error is `Transient`; a refused address, a malformed
    /// reply or any other client error is `Permanent`. No error carries
    /// the API key, whatever the server sends back.
    pub fn complete(&self, prompt: &Prompt<'_>) -> Result<Completion, ClassifiedError> {
        let key = self.api_key()?;
        self.exchange(prompt, key.as_deref())
            .map_err(|e| scrub(e, key.as_deref()))
    }

    fn exchange(
        &self,
        prompt: &Prompt<'_>,
        key: Option<&str>,
    ) -> Result<Completion, ClassifiedError> {
        let end = Instant::now() + self.config.timeout();
        let addrs = self.checked_addrs(end)?;
        let stream = connect(&addrs, end)?;
        let mut transport = SyncTransport::new(Deadline { stream, end });
        let body = request_body(self.config.model(), prompt, self.config.json_mode());
        let head = self.request_head(body.len(), key).map_err(permanent)?;
        transport
            .write_request_head(&head)
            .map_err(|e| self.io(e))?;
        transport
            .write_body(body.as_bytes())
            .map_err(|e| self.io(e))?;
        let response = self.final_head(&mut transport)?;
        let framing = body::response_framing(&response.headers, &Method::Post, response.status)
            .map_err(|e| permanent(format!("bad response framing: {e}")))?;
        let raw = transport
            .read_request_body(framing, DEFAULT_MAX_BODY_LEN)
            .map_err(|e| self.io(e))?;
        // Redact before `check_status` cuts an excerpt: a key split at the
        // cut would no longer match the outer scrub.
        check_status(
            response.status,
            &redact(&String::from_utf8_lossy(&raw), key),
        )?;
        let text = String::from_utf8(raw)
            .map_err(|_| permanent("the reply is not valid UTF-8".to_owned()))?;
        parse_response(&text).map_err(permanent)
    }

    /// The final response head, read past interim 1xx responses such as
    /// `103 Early Hints`. A protocol switch is refused.
    fn final_head(
        &self,
        transport: &mut SyncTransport<Deadline>,
    ) -> Result<ResponseHead, ClassifiedError> {
        for _ in 0..=MAX_INTERIM {
            let head = transport
                .read_response_head(DEFAULT_MAX_HEAD_LEN)
                .map_err(|e| self.io(e))?;
            if head.status.as_u16() == 101 {
                return Err(permanent(
                    "unexpected protocol switch (HTTP 101)".to_owned(),
                ));
            }
            if !head.status.is_informational() {
                return Ok(head);
            }
        }
        Err(permanent(format!(
            "more than {MAX_INTERIM} interim responses"
        )))
    }

    fn api_key(&self) -> Result<Option<String>, ClassifiedError> {
        let Some(name) = self.config.api_key_env() else {
            return Ok(None);
        };
        match (self.env)(name) {
            Some(key) if !key.is_empty() => Ok(Some(key)),
            _ => Err(ClassifiedError::Unavailable(err(format!(
                "{name} is not set"
            )))),
        }
    }

    fn checked_addrs(&self, end: Instant) -> Result<Vec<SocketAddr>, ClassifiedError> {
        let host = self.config.resolve_host();
        let resolved = self.lookup(host, end)?.map_err(|e| {
            ClassifiedError::Unavailable(err(format!("cannot resolve {host}: {e}")))
        })?;
        all_private(resolved).map_err(|bad| match bad {
            Some(addr) => permanent(format!(
                "refusing {host}: it resolves to {}, which is not loopback or private",
                addr.ip()
            )),
            None => permanent(format!("refusing {host}: it resolves to no address")),
        })
    }

    /// Resolve `host` before `end`. An IP literal needs no lookup.
    fn lookup(
        &self,
        host: &str,
        end: Instant,
    ) -> Result<io::Result<Vec<SocketAddr>>, ClassifiedError> {
        let port = self.config.base().port;
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(Ok(vec![SocketAddr::new(ip, port)]));
        }
        let slot = Pending::claim(&self.pending).ok_or_else(|| {
            ClassifiedError::Transient(err(format!(
                "{MAX_PENDING_LOOKUPS} lookups still pending; DNS is not answering"
            )))
        })?;
        let (tx, rx) = mpsc::channel();
        let resolver = Arc::clone(&self.resolver);
        let name = host.to_owned();
        thread::Builder::new()
            .name("orch-openai-dns".to_owned())
            .spawn(move || {
                let _slot = slot;
                // The receiver is gone once the deadline passed; nothing to do.
                let _ = tx.send(resolver.resolve(&name, port));
            })
            .map_err(|e| ClassifiedError::Transient(err(format!("cannot start lookup: {e}"))))?;
        let left = remaining(end).map_err(|_| timed_out())?;
        rx.recv_timeout(left).map_err(|_| timed_out())
    }

    fn request_head(&self, len: usize, key: Option<&str>) -> Result<RequestHead, String> {
        let base = self.config.base();
        let mut headers = HeaderMap::new();
        let fail = |e: rusty_http::Error| e.to_string();
        headers.insert("Host", &base.host_header()).map_err(fail)?;
        headers
            .insert("Content-Type", "application/json")
            .map_err(fail)?;
        headers
            .insert("Content-Length", &len.to_string())
            .map_err(fail)?;
        headers.insert("Accept", "application/json").map_err(fail)?;
        headers.insert("Connection", "close").map_err(fail)?;
        if let Some(key) = key {
            headers
                .insert_sensitive("Authorization", &format!("Bearer {key}"))
                // The message would name the header, never the value; keep
                // it generic anyway so a key cannot leak through it.
                .map_err(|_| "the API key is not a valid header value".to_owned())?;
        }
        Ok(RequestHead {
            method: Method::Post,
            target: self.config.completions_target(),
            version: Version::Http11,
            headers,
        })
    }

    fn io(&self, e: TransportError) -> ClassifiedError {
        match e {
            TransportError::Io(e) if is_timeout(&e) => ClassifiedError::Transient(err(format!(
                "timed out after {}s",
                self.config.timeout().as_secs_f32()
            ))),
            TransportError::Io(e) => ClassifiedError::Transient(err(format!("connection: {e}"))),
            TransportError::Http(e) => permanent(format!("malformed HTTP reply: {e}")),
        }
    }
}

fn connect(addrs: &[SocketAddr], end: Instant) -> Result<TcpStream, ClassifiedError> {
    let mut last = None;
    for addr in addrs {
        let left = remaining(end).map_err(|_| timed_out())?;
        match TcpStream::connect_timeout(addr, left) {
            Ok(stream) => return Ok(stream),
            Err(e) => last = Some((addr, e)),
        }
    }
    Err(match last {
        Some((_, e)) if is_timeout(&e) => timed_out(),
        Some((addr, e)) => {
            ClassifiedError::Unavailable(err(format!("cannot connect to {addr}: {e}")))
        }
        None => permanent("no address to connect to".to_owned()),
    })
}

fn check_status(status: StatusCode, body: &str) -> Result<(), ClassifiedError> {
    let code = status.as_u16();
    if status.is_success() {
        return Ok(());
    }
    let detail = format!("HTTP {code}: {}", excerpt(body));
    Err(match code {
        401 | 403 | 404 => ClassifiedError::Unavailable(err(detail)),
        408 | 429 => ClassifiedError::Transient(err(detail)),
        _ if status.is_server_error() => ClassifiedError::Transient(err(detail)),
        _ => permanent(detail),
    })
}

/// `e` with every copy of the key masked: a server can echo the request
/// back in a body, a header or a framing line.
fn scrub(e: ClassifiedError, key: Option<&str>) -> ClassifiedError {
    let clean = |m: AgentError| AgentError(redact(&m.0, key));
    match e {
        ClassifiedError::Transient(m) => ClassifiedError::Transient(clean(m)),
        ClassifiedError::Permanent(m) => ClassifiedError::Permanent(clean(m)),
        ClassifiedError::Unavailable(m) => ClassifiedError::Unavailable(clean(m)),
    }
}

/// `body` with every copy of the key masked.
fn redact(body: &str, key: Option<&str>) -> String {
    match key {
        Some(key) if !key.is_empty() => body.replace(key, "[redacted]"),
        _ => body.to_owned(),
    }
}

/// At most 200 characters of a server's error body, on one line.
fn excerpt(body: &str) -> String {
    let flat: String = body
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(200)
        .collect();
    flat.trim().to_owned()
}

fn remaining(end: Instant) -> io::Result<Duration> {
    end.checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| io::Error::new(ErrorKind::TimedOut, "deadline passed"))
}

fn is_timeout(e: &io::Error) -> bool {
    matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock)
}

fn err(message: String) -> AgentError {
    AgentError(format!("openai-http: {message}"))
}

fn permanent(message: String) -> ClassifiedError {
    ClassifiedError::Permanent(err(message))
}

fn timed_out() -> ClassifiedError {
    ClassifiedError::Transient(err("timed out".to_owned()))
}

/// One running lookup's place in its client's count, released when the
/// lookup thread ends, not when its caller gives up.
struct Pending(Arc<AtomicUsize>);

impl Pending {
    fn claim(count: &Arc<AtomicUsize>) -> Option<Self> {
        count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_PENDING_LOOKUPS).then_some(n + 1)
            })
            .ok()
            .map(|_| Self(Arc::clone(count)))
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A stream whose every read and write is bounded by one end time, so a
/// server that trickles bytes cannot hold the call past its deadline.
struct Deadline {
    stream: TcpStream,
    end: Instant,
}

impl Read for Deadline {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(remaining(self.end)?))?;
        self.stream.read(buf)
    }
}

impl Write for Deadline {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(remaining(self.end)?))?;
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_classes() {
        let class = |code| match check_status(StatusCode::from_u16(code), "x") {
            Ok(()) => "ok",
            Err(ClassifiedError::Transient(_)) => "transient",
            Err(ClassifiedError::Permanent(_)) => "permanent",
            Err(ClassifiedError::Unavailable(_)) => "unavailable",
        };
        let cases = [
            (200, "ok"),
            (401, "unavailable"),
            (403, "unavailable"),
            (404, "unavailable"),
            (408, "transient"),
            (429, "transient"),
            (500, "transient"),
            (503, "transient"),
            (400, "permanent"),
            (422, "permanent"),
        ];
        for (code, expected) in cases {
            assert_eq!(class(code), expected, "{code}");
        }
    }

    #[test]
    fn redaction_masks_every_copy_of_the_key() {
        assert_eq!(
            redact("k sk-1 and sk-1", Some("sk-1")),
            "k [redacted] and [redacted]"
        );
        assert_eq!(redact("body", None), "body");
    }

    #[test]
    fn excerpts_are_short_and_flat() {
        assert_eq!(excerpt("a\nb\r\n"), "a b");
        assert_eq!(excerpt(&"z".repeat(500)).len(), 200);
    }
}

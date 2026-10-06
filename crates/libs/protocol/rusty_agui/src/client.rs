//! An AG-UI client: `POST` a [`RunAgentInput`], read the event stream.
//!
//! Blocking, on `rusty_http`'s sync transport over `std::net`, the mirror
//! of [`crate::serve`]: no async runtime, no TLS (plain `http://` only,
//! which is what a gateway or a routine on the same host needs). Each run
//! is one connection. The events come back through a [`Verifier`], so a
//! consumer sees canonical, ordered events or an error naming the rule
//! the agent broke, never a half-formed stream.
//!
//! ```no_run
//! use rusty_agui::{HttpAgent, Message, Reducer, RunAgentInput};
//!
//! let agent = HttpAgent::new("http://127.0.0.1:8080/api/agent")?;
//! let input = RunAgentInput::new("thread-1", "run-1", vec![Message::user("u1", "hello")]);
//! let mut view = Reducer::from_input(&input);
//! for event in agent.run(&input)? {
//!     view.apply(&event?)?;
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::event::Event;
use crate::sse;
use crate::types::RunAgentInput;
use crate::verify::Verifier;
use crate::{Error, Result};
use rusty_http::body::response_framing;
use rusty_http::head::RequestHead;
use rusty_http::sync::{BodyReader, SyncTransport};
use rusty_http::{HeaderMap, Method, Url, Version};
use std::collections::VecDeque;
use std::net::TcpStream;
use std::time::Duration;

const MAX_HEAD_BYTES: usize = 16 * 1024;

/// A remote AG-UI agent, addressed by URL.
#[derive(Clone, Debug)]
pub struct HttpAgent {
    url: Url,
    headers: Vec<(String, String)>,
    timeout: Option<Duration>,
}

impl HttpAgent {
    /// An agent at `url` (`http://host[:port]/path`).
    pub fn new(url: &str) -> Result<Self> {
        let url = Url::parse(url).map_err(|e| Error::Transport(e.to_string()))?;
        if url.scheme != "http" {
            return Err(Error::Transport(format!(
                "only http:// is supported, got {}://",
                url.scheme
            )));
        }
        Ok(Self {
            url,
            headers: Vec::new(),
            timeout: Some(Duration::from_secs(300)),
        })
    }

    /// Adds a request header (an `Authorization`, say) to every run.
    #[must_use]
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// The longest silence tolerated while waiting for the next bytes;
    /// `None` waits forever. Default five minutes.
    #[must_use]
    pub fn read_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    /// Starts a run and returns its events as they arrive. The request
    /// is sent and the response head read before this returns, so a
    /// refused run (a non-200, or a body that is not an event stream) is
    /// an error here, not on the first event.
    pub fn run(&self, input: &RunAgentInput) -> Result<RunStream> {
        let body = input.to_value().to_json_string().into_bytes();
        let stream = TcpStream::connect((self.url.host.as_str(), self.url.port))
            .map_err(|e| Error::Transport(format!("connect {}: {e}", self.url.host_header())))?;
        stream
            .set_read_timeout(self.timeout)
            .map_err(|e| Error::Transport(e.to_string()))?;
        let mut transport = SyncTransport::new(stream);

        let mut headers = HeaderMap::new();
        let _ = headers.insert("Host", &self.url.host_header());
        let _ = headers.insert("Content-Type", "application/json");
        let _ = headers.insert("Accept", sse::CONTENT_TYPE);
        let _ = headers.insert("Content-Length", &body.len().to_string());
        let _ = headers.insert("Connection", "close");
        for (name, value) in &self.headers {
            let _ = headers.insert(name, value);
        }
        transport
            .write_request_head(&RequestHead {
                method: Method::Post,
                target: self.url.request_target(),
                version: Version::Http11,
                headers,
            })
            .and_then(|()| transport.write_body(&body))
            .map_err(|e| Error::Transport(e.to_string()))?;

        let head = transport
            .read_response_head(MAX_HEAD_BYTES)
            .map_err(|e| Error::Transport(e.to_string()))?;
        let framing = response_framing(&head.headers, &Method::Post, head.status)
            .map_err(|e| Error::Transport(e.to_string()))?;
        let mut reader = transport.into_body_reader(framing);
        if head.status.as_u16() != 200 {
            return Err(Error::Status {
                status: head.status.as_u16(),
                body: read_all(&mut reader),
            });
        }
        let content_type = head.headers.get("content-type").unwrap_or("");
        if !content_type.starts_with(sse::CONTENT_TYPE) {
            return Err(Error::Transport(format!(
                "expected {}, got {content_type:?}",
                sse::CONTENT_TYPE
            )));
        }
        Ok(RunStream {
            reader,
            decoder: sse::Decoder::new(),
            verifier: Verifier::new(),
            pending: VecDeque::new(),
            done: false,
        })
    }
}

/// Reads whatever body is there, lossily, for an error message.
fn read_all(reader: &mut BodyReader<TcpStream>) -> String {
    let mut bytes = Vec::new();
    while let Ok(Some(chunk)) = reader.next_chunk() {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > 64 * 1024 {
            break;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The events of one run, verified and in order. Ends after
/// `RUN_FINISHED` or `RUN_ERROR`, or with an error when the stream closes
/// before either, breaks the ordering rules, or is not AG-UI JSON.
pub struct RunStream {
    reader: BodyReader<TcpStream>,
    decoder: sse::Decoder,
    verifier: Verifier,
    pending: VecDeque<Event>,
    done: bool,
}

impl RunStream {
    fn fill(&mut self) -> Result<()> {
        loop {
            if !self.pending.is_empty() || self.done {
                return Ok(());
            }
            let chunk = self
                .reader
                .next_chunk()
                .map_err(|e| Error::Transport(e.to_string()))?;
            let raw = match chunk {
                Some(bytes) => self.decoder.feed(&bytes)?,
                None => {
                    self.done = true;
                    let last = self.decoder.finish()?;
                    let events: Vec<Event> = last.into_iter().collect();
                    self.verify_into_pending(events)?;
                    if !self.verifier.is_finished() {
                        return Err(Error::Transport(
                            "stream closed before RUN_FINISHED or RUN_ERROR".into(),
                        ));
                    }
                    return Ok(());
                }
            };
            self.verify_into_pending(raw)?;
            if self.verifier.is_finished() {
                self.done = true;
            }
        }
    }

    fn verify_into_pending(&mut self, events: Vec<Event>) -> Result<()> {
        for event in events {
            self.pending.extend(self.verifier.push(event)?);
        }
        Ok(())
    }
}

impl Iterator for RunStream {
    type Item = Result<Event>;

    fn next(&mut self) -> Option<Result<Event>> {
        if let Err(e) = self.fill() {
            self.done = true;
            self.pending.clear();
            return Some(Err(e));
        }
        self.pending.pop_front().map(Ok)
    }
}

#[cfg(all(test, feature = "serve"))]
mod tests {
    use super::*;
    use crate::serve::{Agent, AgentHandler, Emitter};
    use crate::{EventKind, Message, Reducer, RunOutcome};
    use rusty_json::{json, Value};
    use std::net::SocketAddr;

    struct Echo;

    impl Agent for Echo {
        fn run(&mut self, input: &RunAgentInput, out: &mut Emitter<'_>) -> Result<Option<Value>> {
            let last = input.messages.last().map(|m| match m {
                Message::User { content, .. } => content.text(),
                _ => String::new(),
            });
            if last.as_deref() == Some("fail") {
                return Err(Error::Agent("asked to fail".into()));
            }
            out.state(json!({"n": 0}))?;
            out.emit(EventKind::StateDelta {
                delta: json!([{"op": "replace", "path": "/n", "value": 1}]),
            })?;
            out.text(&format!("echo: {}", last.unwrap_or_default()))?;
            Ok(Some(json!("done")))
        }
    }

    fn start() -> (SocketAddr, rusty_serve::ShutdownHandle) {
        let server =
            rusty_serve::Server::bind("127.0.0.1:0".parse().unwrap(), AgentHandler::new(Echo))
                .unwrap();
        let addr = server.local_addr().unwrap();
        let stop = server.shutdown_handle().unwrap();
        std::thread::spawn(move || server.run().unwrap());
        (addr, stop)
    }

    #[test]
    fn client_and_server_round_trip_through_the_reducer() {
        let (addr, stop) = start();
        let agent = HttpAgent::new(&format!("http://{addr}/api/agent")).unwrap();
        let input = RunAgentInput::new("t", "r", vec![Message::user("u", "hi")]);
        let mut view = Reducer::from_input(&input);
        let mut kinds = Vec::new();
        for event in agent.run(&input).unwrap() {
            let event = event.unwrap();
            kinds.push(event.kind.type_name());
            view.apply(&event).unwrap();
        }
        assert_eq!(
            kinds,
            [
                "RUN_STARTED",
                "STATE_SNAPSHOT",
                "STATE_DELTA",
                "TEXT_MESSAGE_START",
                "TEXT_MESSAGE_CONTENT",
                "TEXT_MESSAGE_END",
                "RUN_FINISHED"
            ]
        );
        assert_eq!(view.state, json!({"n": 1}));
        assert_eq!(view.messages.len(), 2);
        assert_eq!(view.messages[1], Message::assistant("1", "echo: hi"));
        stop.shutdown();
    }

    #[test]
    fn an_agent_failure_arrives_as_run_error_not_a_client_error() {
        let (addr, stop) = start();
        let agent = HttpAgent::new(&format!("http://{addr}/api/agent")).unwrap();
        let input = RunAgentInput::new("t", "r", vec![Message::user("u", "fail")]);
        let events: Vec<Event> = agent.run(&input).unwrap().map(Result::unwrap).collect();
        assert!(matches!(
            &events.last().unwrap().kind,
            EventKind::RunError { message, .. } if message == "asked to fail"
        ));
        let finished = events.iter().any(|e| {
            matches!(
                &e.kind,
                EventKind::RunFinished {
                    outcome: Some(RunOutcome::Success),
                    ..
                }
            )
        });
        assert!(!finished);
        stop.shutdown();
    }

    /// A handler that is not an agent: answers every request with 404.
    struct Refuse;

    impl rusty_serve::Handler for Refuse {
        fn handle(&mut self, _: &rusty_serve::Request<'_>) -> rusty_serve::Response {
            rusty_serve::Response::json(
                rusty_http::StatusCode::NOT_FOUND,
                br#"{"error":"no agent here"}"#.to_vec(),
            )
        }
    }

    #[test]
    fn refusals_and_transport_failures_are_typed_errors() {
        let server = rusty_serve::Server::bind("127.0.0.1:0".parse().unwrap(), Refuse).unwrap();
        let addr = server.local_addr().unwrap();
        let stop = server.shutdown_handle().unwrap();
        std::thread::spawn(move || server.run().unwrap());
        let input = RunAgentInput::new("t", "r", vec![]);

        let refused = HttpAgent::new(&format!("http://{addr}/api/agent")).unwrap();
        match refused.run(&input) {
            Err(Error::Status { status, body }) => {
                assert_eq!(status, 404);
                assert!(body.contains("no agent here"), "{body}");
            }
            Err(other) => panic!("expected a status error, got {other}"),
            Ok(_) => panic!("expected a status error, got a stream"),
        }
        stop.shutdown();

        assert!(matches!(
            HttpAgent::new("https://example.invalid/x"),
            Err(Error::Transport(_))
        ));
        let closed = HttpAgent::new("http://127.0.0.1:1/x").unwrap();
        assert!(matches!(closed.run(&input), Err(Error::Transport(_))));
    }
}

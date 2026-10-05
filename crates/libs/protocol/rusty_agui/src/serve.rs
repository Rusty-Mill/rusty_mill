//! An AG-UI run endpoint for [`rusty_serve`]'s blocking server.
//!
//! An [`Agent`] implements one method: given a [`RunAgentInput`] and an
//! [`Emitter`], do the work and return the run's result. The
//! [`AgentHandler`] does the protocol: it parses the request, frames the
//! run with `RUN_STARTED` and `RUN_FINISHED` (or `RUN_ERROR` when the
//! agent fails or emits a bad sequence), verifies every event through a
//! [`Verifier`], and streams the frames as `text/event-stream`.
//!
//! The agent runs on its own thread while the connection thread writes
//! what it emits, so a slow agent streams rather than buffers. One agent
//! serves one run at a time (it is behind a mutex); a second request
//! waits. That matches `rusty_serve`'s one-store-one-lock model and is
//! enough for a personal server.

use crate::event::{Event, EventKind, RunOutcome};
use crate::types::{Role, RunAgentInput};
use crate::verify::Verifier;
use crate::{sse, Error, Result};
use rusty_http::{Method, StatusCode};
use rusty_json::Value;
use rusty_serve::{Handler, Request, Response};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

/// Something that answers a run by emitting events.
pub trait Agent: Send + 'static {
    /// Handles one run. Emit through `out`; return the run's result, or an
    /// error that becomes `RUN_ERROR` ([`Error::Agent`] for the agent's
    /// own failures). Do not emit `RUN_STARTED` or `RUN_FINISHED`: the
    /// handler frames the run.
    fn run(&mut self, input: &RunAgentInput, out: &mut Emitter<'_>) -> Result<Option<Value>>;
}

/// What an [`Agent`] emits through: every event is verified, then sent.
pub struct Emitter<'a> {
    verifier: &'a mut Verifier,
    sink: &'a mut dyn FnMut(Event) -> Result<()>,
    next_id: u64,
}

impl Emitter<'_> {
    /// Emits one event. An ordering violation is returned and also ends
    /// the run.
    pub fn emit(&mut self, event: impl Into<Event>) -> Result<()> {
        for event in self.verifier.push(event.into())? {
            (self.sink)(event)?;
        }
        Ok(())
    }

    /// A fresh id, unique within this run, for messages and tool calls.
    pub fn next_id(&mut self) -> String {
        self.next_id += 1;
        format!("{}", self.next_id)
    }

    /// Emits a complete assistant text message and returns its id.
    pub fn text(&mut self, text: &str) -> Result<String> {
        let message_id = self.next_id();
        self.emit(EventKind::TextMessageStart {
            message_id: message_id.clone(),
            role: Role::Assistant,
        })?;
        if !text.is_empty() {
            self.emit(EventKind::TextMessageContent {
                message_id: message_id.clone(),
                delta: text.into(),
            })?;
        }
        self.emit(EventKind::TextMessageEnd {
            message_id: message_id.clone(),
        })?;
        Ok(message_id)
    }

    /// Emits a state snapshot.
    pub fn state(&mut self, snapshot: Value) -> Result<()> {
        self.emit(EventKind::StateSnapshot { snapshot })
    }
}

/// Runs an [`Agent`] for every `POST` it is given.
///
/// Mount it as the whole [`Handler`], or keep your own handler and call
/// [`AgentHandler::handle_run`] for the route that is the agent.
pub struct AgentHandler<A> {
    agent: Arc<Mutex<A>>,
}

impl<A: Agent> AgentHandler<A> {
    /// Wraps an agent.
    pub fn new(agent: A) -> Self {
        Self {
            agent: Arc::new(Mutex::new(agent)),
        }
    }

    /// Answers one run request: `body` is a JSON [`RunAgentInput`]; the
    /// response is the event stream, or a `400` for a body that is not a
    /// run input.
    pub fn handle_run(&self, body: &[u8]) -> Response {
        let input = match std::str::from_utf8(body)
            .map_err(|e| Error::Json(e.to_string()))
            .and_then(RunAgentInput::from_json)
        {
            Ok(input) => input,
            Err(e) => return bad_request(&e.to_string()),
        };
        // Bounded so a client that stops reading stalls the agent instead
        // of growing memory without limit.
        let (tx, rx) = sync_channel::<Vec<u8>>(64);
        let agent = Arc::clone(&self.agent);
        std::thread::spawn(move || run_framed(agent, input, tx));
        Response::stream(sse::CONTENT_TYPE, Frames(rx))
    }
}

impl<A: Agent> Handler for AgentHandler<A> {
    fn handle(&mut self, request: &Request<'_>) -> Response {
        if request.method != &Method::Post {
            return Response::json(
                StatusCode::METHOD_NOT_ALLOWED,
                br#"{"error":{"code":"method_not_allowed","message":"POST a RunAgentInput"}}"#
                    .to_vec(),
            );
        }
        self.handle_run(request.body)
    }
}

struct Frames(Receiver<Vec<u8>>);

impl Iterator for Frames {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        self.0.recv().ok()
    }
}

/// Frames and verifies one run, sending encoded frames to `tx` until the
/// run ends or the receiver is gone.
fn run_framed<A: Agent>(agent: Arc<Mutex<A>>, input: RunAgentInput, tx: SyncSender<Vec<u8>>) {
    let mut verifier = Verifier::new();
    let send = |event: Event| -> Result<()> {
        tx.send(sse::encode(&event).into_bytes())
            .map_err(|_| Error::Closed)
    };
    let (thread_id, run_id) = (input.thread_id.clone(), input.run_id.clone());
    let started: Event = EventKind::RunStarted {
        thread_id: thread_id.clone(),
        run_id: run_id.clone(),
        parent_run_id: input.parent_run_id.clone(),
        input: None,
    }
    .into();
    let emit = |verifier: &mut Verifier, event: Event| -> Result<()> {
        for event in verifier.push(event)? {
            send(event)?;
        }
        Ok(())
    };
    if emit(&mut verifier, started).is_err() {
        return;
    }

    let outcome = {
        let mut agent = match agent.lock() {
            Ok(agent) => agent,
            Err(_) => {
                let _ = emit(
                    &mut verifier,
                    run_error("agent unavailable", Some("poisoned")),
                );
                return;
            }
        };
        let mut sink = |event: Event| send(event);
        let mut emitter = Emitter {
            verifier: &mut verifier,
            sink: &mut sink,
            next_id: 0,
        };
        agent.run(&input, &mut emitter)
    };

    let closing = match outcome {
        Ok(result) if !verifier.is_finished() => EventKind::RunFinished {
            thread_id,
            run_id,
            result,
            outcome: Some(RunOutcome::Success),
        }
        .into(),
        Ok(_) => return,
        Err(Error::Closed) => return,
        Err(error) if !verifier.is_finished() => run_error(&error_message(&error), None),
        Err(_) => return,
    };
    let _ = emit(&mut verifier, closing);
}

/// What `RUN_ERROR` says: the agent's own words for its failure, the
/// error's description otherwise.
fn error_message(error: &Error) -> String {
    match error {
        Error::Agent(message) => message.clone(),
        other => other.to_string(),
    }
}

fn run_error(message: &str, code: Option<&str>) -> Event {
    EventKind::RunError {
        message: message.into(),
        code: code.map(String::from),
    }
    .into()
}

fn bad_request(message: &str) -> Response {
    let mut body = Value::object();
    let mut error = Value::object();
    error.insert("code", "bad_request");
    error.insert("message", message);
    body.insert("error", error);
    Response::json(StatusCode::BAD_REQUEST, body.to_json_string().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Reducer;
    use rusty_json::json;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    /// Echoes the last user message, streams a state delta, and fails on
    /// the word "fail".
    struct Echo;

    impl Agent for Echo {
        fn run(&mut self, input: &RunAgentInput, out: &mut Emitter<'_>) -> Result<Option<Value>> {
            let last = input
                .messages
                .last()
                .map(|m| match m {
                    crate::Message::User { content, .. } => content.text(),
                    _ => String::new(),
                })
                .unwrap_or_default();
            if last == "fail" {
                return Err(Error::Agent("asked to fail".into()));
            }
            if last == "break" {
                // A bad sequence: content for a message never started.
                out.emit(EventKind::TextMessageContent {
                    message_id: "ghost".into(),
                    delta: "x".into(),
                })?;
            }
            out.state(json!({"turns": 0}))?;
            out.emit(EventKind::StateDelta {
                delta: json!([{"op": "replace", "path": "/turns", "value": 1}]),
            })?;
            out.text(&format!("you said: {last}"))?;
            Ok(Some(json!({"echoed": last})))
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

    fn post(addr: SocketAddr, body: &str) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        write!(
            stream,
            "POST /api/agent HTTP/1.1\r\nHost: t\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        out
    }

    /// Decodes a chunked SSE response into events via the SSE decoder.
    fn events_of(raw: &str) -> Vec<Event> {
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        assert!(head.contains("200 OK"), "{head}");
        assert!(head.contains("Content-Type: text/event-stream"), "{head}");
        assert!(head.contains("Transfer-Encoding: chunked"), "{head}");
        let mut transport =
            rusty_http::sync::SyncTransport::new(std::io::Cursor::new(body.as_bytes().to_vec()));
        let mut decoder = sse::Decoder::new();
        let mut events = Vec::new();
        let bytes = transport.read_chunked_body(1024).unwrap();
        events.extend(decoder.feed(&bytes).unwrap());
        if let Some(last) = decoder.finish().unwrap() {
            events.push(last);
        }
        events
    }

    fn input(text: &str) -> String {
        json!({"threadId": "t", "runId": "r", "messages": [{"id": "u1", "role": "user", "content": text}]})
            .to_json_string()
    }

    #[test]
    fn a_run_is_framed_verified_and_reducible() {
        let (addr, stop) = start();
        let events = events_of(&post(addr, &input("hello")));
        let kinds: Vec<_> = events.iter().map(|e| e.kind.type_name()).collect();
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
        let mut reducer = Reducer::new();
        reducer.apply_all(&events).unwrap();
        assert_eq!(reducer.state, json!({"turns": 1}));
        assert_eq!(
            reducer.messages,
            vec![crate::Message::assistant("1", "you said: hello")]
        );
        assert!(matches!(
            &events.last().unwrap().kind,
            EventKind::RunFinished { result: Some(r), outcome: Some(RunOutcome::Success), .. }
                if *r == json!({"echoed": "hello"})
        ));
        stop.shutdown();
    }

    #[test]
    fn an_agent_error_and_a_bad_sequence_both_end_in_run_error() {
        let (addr, stop) = start();
        let failed = events_of(&post(addr, &input("fail")));
        assert!(matches!(
            &failed.last().unwrap().kind,
            EventKind::RunError { message, .. } if message == "asked to fail"
        ));
        let broken = events_of(&post(addr, &input("break")));
        assert_eq!(
            broken.len(),
            2,
            "RUN_STARTED then RUN_ERROR; the bad event never left"
        );
        assert!(matches!(
            &broken[1].kind,
            EventKind::RunError { message, .. } if message.contains("not open")
        ));
        stop.shutdown();
    }

    #[test]
    fn bad_requests_are_refused() {
        let (addr, stop) = start();
        let raw = post(addr, "{\"runId\":\"r\"}");
        assert!(raw.starts_with("HTTP/1.1 400"), "{raw}");
        assert!(raw.contains("missing \\\"threadId\\\""), "{raw}");
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .write_all(b"GET /api/agent HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        assert!(out.starts_with("HTTP/1.1 405"), "{out}");
        stop.shutdown();
    }
}

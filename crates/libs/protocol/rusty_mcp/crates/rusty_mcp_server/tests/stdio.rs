//! The stdio transport over in-memory pipes.

use rusty_mcp_proto::jsonrpc::code;
use rusty_mcp_proto::{CallToolResult, Message, RequestId, Schema, Tool};
use rusty_mcp_server::stdio::{serve, Options};
use rusty_mcp_server::{Dispatcher, Server};
use std::io::{self, BufReader, Cursor, Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A writer whose contents the test can read afterwards.
#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("lock").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Shared {
    fn lines(&self) -> Vec<Message> {
        String::from_utf8(self.0.lock().expect("lock").clone())
            .expect("utf8")
            .lines()
            .map(|l| Message::from_json(l).expect("server wrote valid JSON-RPC"))
            .collect()
    }
}

/// A reader fed from the test thread, so it can send input while a handler runs.
struct Feed(Receiver<Vec<u8>>, Vec<u8>);

impl Read for Feed {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.1.is_empty() {
            match self.0.recv() {
                Ok(bytes) => self.1 = bytes,
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.1.len());
        out[..n].copy_from_slice(&self.1[..n]);
        self.1.drain(..n);
        Ok(n)
    }
}

fn feed() -> (Sender<Vec<u8>>, BufReader<Feed>) {
    let (tx, rx) = mpsc::channel();
    (tx, BufReader::new(Feed(rx, Vec::new())))
}

fn server() -> Dispatcher<Server> {
    Dispatcher::new(
        Server::new("t", "1")
            .tool(Tool::new("echo", "e", Schema::object().build()), |_, _| {
                Ok(CallToolResult::text("hi"))
            })
            .tool(
                Tool::new("slow", "s", Schema::object().build()),
                |ctx, _| {
                    let start = Instant::now();
                    while start.elapsed() < Duration::from_secs(10) {
                        if ctx.is_cancelled() {
                            return Ok(CallToolResult::text("cancelled"));
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Ok(CallToolResult::text("finished"))
                },
            ),
    )
}

#[test]
fn a_session_of_requests_is_answered_in_order() {
    let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n\n\
                 {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                 {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"echo\"}}\r\n";
    let out = Shared::default();
    serve(
        server(),
        Cursor::new(input.as_bytes().to_vec()),
        out.clone(),
        Options::default(),
    )
    .expect("serves to end of input");
    let replies = out.lines();
    assert_eq!(replies.len(), 2, "{replies:?}");
    assert!(matches!(
        &replies[0],
        Message::Response {
            id: RequestId::Number(1),
            ..
        }
    ));
    let Message::Response { result, .. } = &replies[1] else {
        panic!("{:?}", replies[1]);
    };
    assert_eq!(
        CallToolResult::from_value(result).expect("valid"),
        CallToolResult::text("hi")
    );
}

#[test]
fn bad_input_gets_an_error_and_the_server_keeps_going() {
    let input = "not json\n{\"jsonrpc\":\"1.0\",\"id\":1,\"method\":\"x\"}\n{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}\n";
    let out = Shared::default();
    serve(
        server(),
        Cursor::new(input.as_bytes().to_vec()),
        out.clone(),
        Options::default(),
    )
    .expect("serves");
    let replies = out.lines();
    assert!(
        matches!(&replies[0], Message::Error { id: None, error } if error.code == code::PARSE_ERROR)
    );
    assert!(
        matches!(&replies[1], Message::Error { id: None, error } if error.code == code::INVALID_REQUEST)
    );
    assert!(matches!(
        &replies[2],
        Message::Response {
            id: RequestId::Number(3),
            ..
        }
    ));
}

#[test]
fn an_oversized_line_is_refused_without_being_buffered_whole() {
    let big = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"{}\"}}\n",
        "x".repeat(2000)
    );
    let input = format!("{big}{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}}\n");
    let out = Shared::default();
    let options = Options {
        max_line_bytes: 256,
    };
    serve(
        server(),
        Cursor::new(input.into_bytes()),
        out.clone(),
        options,
    )
    .expect("serves");
    let replies = out.lines();
    assert!(
        matches!(&replies[0], Message::Error { id: None, error } if error.message.contains("longer than 256"))
    );
    assert!(matches!(
        &replies[1],
        Message::Response {
            id: RequestId::Number(2),
            ..
        }
    ));
}

#[test]
fn cancelling_a_running_request_stops_its_handler_and_sends_no_response() {
    let (tx, reader) = feed();
    let out = Shared::default();
    let sink = out.clone();
    let worker = std::thread::spawn(move || serve(server(), reader, sink, Options::default()));

    let send = |text: &str| {
        tx.send(format!("{text}\n").into_bytes())
            .expect("server is reading")
    };
    send(r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"slow"}}"#);
    std::thread::sleep(Duration::from_millis(100));
    let cancelled_at = Instant::now();
    send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7}}"#);
    send(r#"{"jsonrpc":"2.0","id":8,"method":"ping"}"#);
    drop(tx);

    worker.join().expect("no panic").expect("serves");
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(2),
        "handler did not stop promptly"
    );
    let replies = out.lines();
    assert_eq!(
        replies.len(),
        1,
        "no response for the cancelled request: {replies:?}"
    );
    assert!(matches!(
        &replies[0],
        Message::Response {
            id: RequestId::Number(8),
            ..
        }
    ));
}

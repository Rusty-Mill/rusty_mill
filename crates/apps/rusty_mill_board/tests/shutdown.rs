//! A stop with work in flight: a real MCP mutation held past the point where the server switches to
//! an event stream is finished and answered before the stop returns, and a stuck one is ended at the
//! deadline. This runs the same `serve_until_drained` as `main`.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusty_mcp_server::{HttpConfig, HttpHandler};
use rusty_mill_board::api::Shared;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{BoardError, Card, Event};
use rusty_mill_board::drain::{InFlight, begin_stop, serve_until_drained};
use rusty_mill_board::mcp;
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::web::{Auth, Web};
use uuid::Uuid;

/// A store whose `insert` waits at a gate: a mutation that takes as long as the test wants.
struct Held {
    inner: MemStore,
    gate: Arc<(Mutex<bool>, Condvar)>,
    entered: Arc<AtomicBool>,
    inserts: Arc<AtomicUsize>,
    done: Arc<AtomicBool>,
}

impl Store for Held {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        self.inner.get(id)
    }
    fn list(&self) -> Result<Vec<Card>, BoardError> {
        self.inner.list()
    }
    fn insert(&self, card: &Card, event: Event) -> Result<(), BoardError> {
        self.inserts.fetch_add(1, Ordering::SeqCst);
        self.entered.store(true, Ordering::SeqCst);
        let (open, opened) = &*self.gate;
        let (_open, timeout) = opened
            .wait_timeout_while(
                open.lock().expect("gate"),
                Duration::from_secs(30),
                |open| !*open,
            )
            .expect("gate");
        assert!(!timeout.timed_out(), "the test never opened the gate");
        self.inner.insert(card, event)?;
        self.done.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn replace(&self, card: &Card, event: Event) -> Result<(), BoardError> {
        self.inner.replace(card, event)
    }
    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        self.inner.events_since(after, limit)
    }
    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
        self.inner.last_event_id()
    }
}

struct Running {
    addr: SocketAddr,
    stop: rusty_serve::ShutdownHandle,
    gate: Arc<(Mutex<bool>, Condvar)>,
    entered: Arc<AtomicBool>,
    /// Mutations that reached the store.
    inserts: Arc<AtomicUsize>,
    done: Arc<AtomicBool>,
    flight: Arc<InFlight>,
    served: JoinHandle<std::io::Result<bool>>,
}

impl Running {
    fn open_gate(&self) {
        *self.gate.0.lock().expect("gate") = true;
        self.gate.1.notify_all();
    }

    fn wait_until(&self, what: &AtomicBool) {
        let end = Instant::now() + Duration::from_secs(20);
        while !what.load(Ordering::SeqCst) {
            assert!(Instant::now() < end, "never happened");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// The board served as `main` serves it, drained for at most `within` after a stop.
fn start(within: Duration) -> Running {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let (entered, done, inserts) = (Arc::default(), Arc::default(), Arc::default());
    let store = Held {
        inner: MemStore::default(),
        gate: Arc::clone(&gate),
        entered: Arc::clone(&entered),
        inserts: Arc::clone(&inserts),
        done: Arc::clone(&done),
    };
    let board: Arc<Shared> = Arc::new(Board::new(
        Box::new(store) as Box<dyn Store>,
        Roles::default(),
    ));
    let flight: Arc<InFlight> = Arc::default();
    let config = HttpConfig {
        allowed_hosts: vec!["127.0.0.1".into()],
        // The reply becomes an event stream if the call is still running after this.
        sse_after: Duration::from_millis(50),
        ..HttpConfig::default()
    };
    let path = config.path.clone();
    let mcp = HttpHandler::new(
        mcp::server_with(Arc::clone(&board), Arc::clone(&flight)).expect("server"),
        config,
    );
    let web = Web::new(board, mcp, path, vec!["127.0.0.1".into()], Auth::Open)
        .with_in_flight(Arc::clone(&flight));
    let server =
        rusty_serve::Server::bind_shared("127.0.0.1:0".parse().expect("addr"), web).expect("bind");
    let addr = server.local_addr().expect("addr");
    let stop = server.shutdown_handle().expect("handle");
    let drained = Arc::clone(&flight);
    let served = std::thread::spawn(move || serve_until_drained(server, &drained, within));
    Running {
        addr,
        stop,
        gate,
        entered,
        inserts,
        done,
        flight,
        served,
    }
}

/// The raw request for `card_add` over MCP, on a connection that stays open unless `close`.
fn card_add_request(title: &str, close: bool) -> String {
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"card_add","arguments":{{"title":"{title}","actor":"a","role":"human"}}}}}}"#
    );
    format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\n{}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
        if close { "Connection: close\r\n" } else { "" },
        body.len()
    )
}

/// `card_add` over MCP titled "held"; the whole response, however it is framed.
fn card_add(addr: SocketAddr) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut stream = TcpStream::connect(addr).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("timeout");
        stream
            .write_all(card_add_request("held", true).as_bytes())
            .expect("write");
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        out
    })
}

/// A keep-alive connection that has finished one request, so it is open and idle.
fn idle_connection(addr: SocketAddr) -> TcpStream {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .expect("write");
    let mut seen = Vec::new();
    let mut byte = [0u8; 1];
    while !seen.ends_with(b"{\"ok\":true}") {
        assert_eq!(stream.read(&mut byte).expect("read"), 1, "closed early");
        seen.push(byte[0]);
    }
    assert!(seen.starts_with(b"HTTP/1.1 200"));
    stream
}

#[test]
fn a_mutation_held_past_the_stream_threshold_is_finished_and_answered_before_the_stop_returns() {
    let s = start(Duration::from_secs(20));
    let client = card_add(s.addr);
    s.wait_until(&s.entered);
    // Longer than `sse_after`: the server has already switched to an event stream.
    std::thread::sleep(Duration::from_millis(200));

    begin_stop(&s.stop, &s.flight);
    // The mutation is still running, so the stop cannot be over.
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !s.served.is_finished(),
        "the stop must wait for the mutation"
    );
    assert!(!s.done.load(Ordering::SeqCst));

    s.open_gate();
    let drained = s.served.join().expect("serve").expect("io");
    assert!(drained, "everything finished inside the deadline");
    assert!(
        s.done.load(Ordering::SeqCst),
        "the mutation had completed when the stop returned"
    );

    let reply = client.join().expect("client");
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(
        reply.contains("held"),
        "the reply carries the card: {reply}"
    );
    assert!(
        reply.contains("0\r\n\r\n"),
        "the stream was ended, not cut off: {reply:?}"
    );
}

#[test]
fn a_mutation_that_never_finishes_is_ended_at_the_deadline() {
    let s = start(Duration::from_millis(300));
    let client = card_add(s.addr);
    s.wait_until(&s.entered);
    begin_stop(&s.stop, &s.flight);

    let gate = Arc::clone(&s.gate);
    let started = Instant::now();
    let drained = s.served.join().expect("serve").expect("io");
    assert!(!drained, "the deadline ended the wait");
    assert!(started.elapsed() < Duration::from_secs(10), "bounded");
    assert!(!s.done.load(Ordering::SeqCst));

    // Let the held call and its client finish so nothing outlives the test.
    *gate.0.lock().expect("gate") = true;
    gate.1.notify_all();
    let _ = client.join();
}

#[test]
fn a_second_mutation_on_a_connection_that_was_already_open_is_not_run_during_a_stop() {
    let s = start(Duration::from_secs(20));
    let mut idle = idle_connection(s.addr);
    let held = card_add(s.addr);
    s.wait_until(&s.entered);

    begin_stop(&s.stop, &s.flight);
    // The old connection asks for more work: it is neither run nor answered with its result.
    idle.write_all(card_add_request("second", false).as_bytes())
        .expect("write");
    let mut reply = String::new();
    let _ = idle.read_to_string(&mut reply);
    assert!(!reply.contains("second"), "not run: {reply}");
    assert!(
        reply.is_empty() || reply.starts_with("HTTP/1.1 503"),
        "dropped or refused: {reply}"
    );
    assert_eq!(
        s.inserts.load(Ordering::SeqCst),
        1,
        "only the held mutation reached the store"
    );

    s.open_gate();
    assert!(s.served.join().expect("serve").expect("io"));
    assert!(held.join().expect("client").contains("held"));
}

/// Stopping the listener directly, not through `begin_stop`: the drain still closes admission
/// before it waits for anything, so nothing that begins later can outrun the waits.
#[test]
fn the_drain_closes_admission_before_it_waits() {
    let s = start(Duration::from_secs(20));
    let held = card_add(s.addr);
    s.wait_until(&s.entered);
    s.stop.shutdown();

    let end = Instant::now() + Duration::from_secs(5);
    while s.flight.enter().is_some() {
        assert!(
            Instant::now() < end,
            "admission was still open while the drain waited"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !s.served.is_finished(),
        "while the held mutation is still running"
    );

    s.open_gate();
    assert!(s.served.join().expect("serve").expect("io"));
    let _ = held.join();
}

#[test]
fn a_stop_closes_admission_before_it_stops_the_listener() {
    let s = start(Duration::from_secs(5));
    begin_stop(&s.stop, &s.flight);
    assert!(
        s.flight.enter().is_none(),
        "closed by the time the stop returns"
    );
    assert!(s.served.join().expect("serve").expect("io"));
}

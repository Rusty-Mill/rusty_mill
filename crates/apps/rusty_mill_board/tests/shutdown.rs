//! A stop with work in flight: a real MCP mutation held past the point where the server switches to
//! an event stream is finished and answered before the stop returns, and a stuck one is ended at the
//! deadline. This runs the same `serve_until_drained` as `main`.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusty_mcp_server::{HttpConfig, HttpHandler};
use rusty_mill_board::api::Shared;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{BoardError, Card, Event};
use rusty_mill_board::drain::{InFlight, serve_until_drained};
use rusty_mill_board::mcp;
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::web::{Auth, Web};
use uuid::Uuid;

/// A store whose `insert` waits at a gate: a mutation that takes as long as the test wants.
struct Held {
    inner: MemStore,
    gate: Arc<(Mutex<bool>, Condvar)>,
    entered: Arc<AtomicBool>,
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
    done: Arc<AtomicBool>,
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
    let (entered, done) = (Arc::default(), Arc::default());
    let store = Held {
        inner: MemStore::default(),
        gate: Arc::clone(&gate),
        entered: Arc::clone(&entered),
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
    let served = std::thread::spawn(move || serve_until_drained(server, &flight, within));
    Running {
        addr,
        stop,
        gate,
        entered,
        done,
        served,
    }
}

/// `card_add` over MCP; the whole response, however it is framed.
fn card_add(addr: SocketAddr) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"card_add","arguments":{"title":"held","actor":"a","role":"human"}}}"#;
        let mut stream = TcpStream::connect(addr).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("timeout");
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .expect("write");
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        out
    })
}

#[test]
fn a_mutation_held_past_the_stream_threshold_is_finished_and_answered_before_the_stop_returns() {
    let s = start(Duration::from_secs(20));
    let client = card_add(s.addr);
    s.wait_until(&s.entered);
    // Longer than `sse_after`: the server has already switched to an event stream.
    std::thread::sleep(Duration::from_millis(200));

    s.stop.shutdown();
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
    s.stop.shutdown();

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

//! Phase 1 exit criteria (docs/projects/mux/ROADMAP.md), against a real
//! server on a Unix socket with real PTYs running `/bin/sh`.

#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rusty_mux_core::{BlockId, ClientId, SessionId, Size, WindowId};
use rusty_mux_proto::{Features, Message, decode, encode};
use rusty_mux_server::{Config, serve};
use rusty_term_core::{AnsiParser, Grid};
use rusty_tokio::io::UnixListener;
use rusty_tokio::{Builder, LocalOptions};

const PATIENCE: Duration = Duration::from_secs(20);

/// Starts a server on a fresh socket and returns its path.
fn start_server() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("rusty-mux-{}-{n}.sock", std::process::id()));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let bind_path = path.clone();
    std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime");
        runtime.block_on(async move {
            let listener = UnixListener::bind(&bind_path).expect("bind");
            ready_tx.send(()).expect("signal ready");
            let config = Config {
                program: "/bin/sh".into(),
                args: Vec::new(),
                cwd: "/".into(),
            };
            serve(listener, config).await.expect("serve");
        });
    });
    ready_rx.recv_timeout(PATIENCE).expect("server bound");
    path
}

/// A blocking test client that keeps its own terminal per attached block.
struct Client {
    stream: UnixStream,
    buf: Vec<u8>,
    id: ClientId,
    next_req: u64,
    terms: HashMap<BlockId, (Grid, AnsiParser)>,
    /// Raw output received since attach, per block.
    output: HashMap<BlockId, Vec<u8>>,
    ready: HashMap<BlockId, bool>,
    /// Whether output is fed to the terminals. The flood test turns it off so
    /// it measures the server, not this debug-built client's parser.
    parse: bool,
}

impl Client {
    fn connect(path: &PathBuf) -> Self {
        let stream = UnixStream::connect(path).expect("connect");
        stream.set_read_timeout(Some(PATIENCE)).expect("timeout");
        let mut client = Self {
            stream,
            buf: Vec::new(),
            id: ClientId::from_uuid(rusty_uuid::Uuid::nil()),
            next_req: 1,
            terms: HashMap::new(),
            output: HashMap::new(),
            ready: HashMap::new(),
            parse: true,
        };
        client.send(&Message::Hello {
            features: Features::NONE,
        });
        match client.recv() {
            Message::Welcome { client: id, .. } => client.id = id,
            other => panic!("expected Welcome, got {other:?}"),
        }
        client
    }

    fn send(&mut self, message: &Message) {
        self.stream
            .write_all(&encode(message).expect("encode"))
            .expect("write");
    }

    /// Reads one message, applying snapshots and output to its terminals.
    fn recv(&mut self) -> Message {
        loop {
            if let Some((message, used)) = decode(&self.buf).expect("decode") {
                self.buf.drain(..used);
                self.apply(&message);
                return message;
            }
            let mut chunk = [0u8; 65536];
            let n = self.stream.read(&mut chunk).expect("read");
            assert!(n > 0, "server closed the connection");
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    fn apply(&mut self, message: &Message) {
        match message {
            Message::Snapshot { block, data } => {
                let grid = Grid::decode_snapshot(data).expect("snapshot decodes");
                self.terms.insert(*block, (grid, AnsiParser::new()));
                self.output.insert(*block, Vec::new());
                self.ready.insert(*block, false);
            }
            Message::Ready { block } => {
                self.ready.insert(*block, true);
            }
            Message::Output { block, data } => {
                assert_eq!(
                    self.ready.get(block),
                    Some(&true),
                    "output before snapshot and ready"
                );
                let (grid, parser) = self.terms.get_mut(block).expect("attached");
                if self.parse {
                    parser.advance(grid, data);
                }
                self.output
                    .entry(*block)
                    .or_default()
                    .extend_from_slice(data);
            }
            _ => {}
        }
    }

    /// Sends a request and returns the `created` id of its `Done` reply.
    fn request(&mut self, make: impl FnOnce(u64) -> Message) -> Option<rusty_uuid::Uuid> {
        let req = self.next_req;
        self.next_req += 1;
        self.send(&make(req));
        loop {
            match self.recv() {
                Message::Done { req: r, created } if r == req => return created,
                Message::Failed { req: r, reason } if r == req => {
                    panic!("request {req} failed: {reason}")
                }
                _ => {}
            }
        }
    }

    fn input(&mut self, block: BlockId, data: &[u8]) {
        let req = self.next_req;
        self.next_req += 1;
        self.send(&Message::Input {
            req,
            block,
            data: data.to_vec(),
        });
    }

    /// Reads until `block`'s output since attach contains `needle`.
    fn wait_for(&mut self, block: BlockId, needle: &str) {
        let deadline = Instant::now() + PATIENCE;
        while !self.saw(block, needle) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {needle:?}"
            );
            self.recv();
        }
    }

    fn saw(&self, block: BlockId, needle: &str) -> bool {
        let out = self
            .output
            .get(&block)
            .map(Vec::as_slice)
            .unwrap_or_default();
        out.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    fn snapshot(&self, block: BlockId) -> Vec<u8> {
        self.terms[&block].0.encode_snapshot()
    }
}

/// Creates a session, window and block from `client`, attached and leased.
fn block_for(client: &mut Client) -> BlockId {
    let session = SessionId::from_uuid(
        client
            .request(|req| Message::CreateSession {
                req,
                name: "s".into(),
            })
            .expect("id"),
    );
    let window = WindowId::from_uuid(
        client
            .request(|req| Message::CreateWindow {
                req,
                session,
                name: "w".into(),
            })
            .expect("id"),
    );
    let size = Size::new(80, 24).expect("size");
    let block = BlockId::from_uuid(
        client
            .request(|req| Message::CreateBlock { req, window, size })
            .expect("id"),
    );
    client.request(|req| Message::Attach { req, block });
    client.request(|req| Message::AcquireLease { req, block });
    block
}

/// Runs a command whose output differs from its echo, and waits for it.
fn run_and_wait(writer: &mut Client, block: BlockId, n: u32) -> String {
    let marker = format!("MARK{}", n * 2);
    writer.input(block, format!("echo MARK$(({n}*2))\r").as_bytes());
    writer.wait_for(block, &marker);
    marker
}

#[test]
fn two_clients_converge_on_one_pty() {
    let path = start_server();
    let mut a = Client::connect(&path);
    let block = block_for(&mut a);
    let mut b = Client::connect(&path);
    b.request(|req| Message::Attach { req, block });

    let marker = run_and_wait(&mut a, block, 21);
    b.wait_for(block, &marker);
    // Settle: one more round trip so both have seen the prompt that follows.
    let marker = run_and_wait(&mut a, block, 22);
    b.wait_for(block, &marker);
    let quiet = run_and_wait(&mut a, block, 23);
    b.wait_for(block, &quiet);

    assert_eq!(a.snapshot(block), b.snapshot(block));
}

#[test]
fn reattach_restores_the_grid_from_the_snapshot() {
    let path = start_server();
    let mut a = Client::connect(&path);
    let block = block_for(&mut a);
    let mut b = Client::connect(&path);
    b.request(|req| Message::Attach { req, block });
    run_and_wait(&mut a, block, 31);

    b.request(|req| Message::Detach { req });
    run_and_wait(&mut a, block, 32);
    let last = run_and_wait(&mut a, block, 33);

    b.request(|req| Message::Attach { req, block });
    // B's grid now comes only from the snapshot; A's from live output.
    assert!(
        !b.saw(block, &last),
        "output after reattach should be new only"
    );
    let after = run_and_wait(&mut a, block, 34);
    b.wait_for(block, &after);
    assert_eq!(a.snapshot(block), b.snapshot(block));
}

#[test]
fn input_round_trips_while_the_pty_floods() {
    let path = start_server();
    let mut a = Client::connect(&path);
    a.parse = false;
    let block = block_for(&mut a);
    run_and_wait(&mut a, block, 41);

    // A background job floods the PTY for the rest of the test; the shell in
    // the foreground must still read and answer a command.
    a.input(
        block,
        b"(i=0; while [ $i -lt 1000000 ]; do echo flood-$i; i=$((i+1)); done) &\r",
    );
    a.wait_for(block, "flood-1000");
    let started = Instant::now();
    let marker = run_and_wait(&mut a, block, 43);
    let elapsed = started.elapsed();
    eprintln!("round trip under flood: {elapsed:?}");
    a.input(block, b"kill $!\r");
    // Measured at 5-70 ms in a debug build; the bound leaves room for a busy
    // CI runner. Without the pump's yield this took seconds.
    assert!(
        elapsed < Duration::from_secs(1),
        "{marker} took {elapsed:?} under flood"
    );
}

#[test]
fn input_without_the_lease_is_refused() {
    let path = start_server();
    let mut a = Client::connect(&path);
    let block = block_for(&mut a);
    let mut b = Client::connect(&path);
    b.request(|req| Message::Attach { req, block });

    b.send(&Message::Input {
        req: 99,
        block,
        data: b"echo nope\r".to_vec(),
    });
    loop {
        if let Message::Failed { req: 99, reason } = b.recv() {
            assert!(reason.contains("does not hold the input lease"), "{reason}");
            break;
        }
    }
}

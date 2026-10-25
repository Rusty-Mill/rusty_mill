//! Helpers shared by the server's integration tests: a real server on a
//! Unix socket and a blocking test client.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rusty_mux_core::{BlockId, ClientId, Event, SessionId, Size, WindowId};
use rusty_mux_proto::{Cursor, Features, Message, decode, encode};
use rusty_mux_server::{Config, serve};
use rusty_term_core::{AnsiParser, Grid};
use rusty_tokio::io::UnixListener;
use rusty_tokio::{Builder, LocalOptions};

pub const PATIENCE: Duration = Duration::from_secs(20);

/// Starts a server running `/bin/sh` in each block; returns its socket.
pub fn start_server() -> PathBuf {
    start_server_with(Config {
        program: "/bin/sh".into(),
        args: Vec::new(),
        cwd: "/".into(),
    })
}

/// Starts a server on a fresh socket and returns its path.
pub fn start_server_with(config: Config) -> PathBuf {
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
            serve(listener, config).await.expect("serve");
        });
    });
    ready_rx.recv_timeout(PATIENCE).expect("server bound");
    path
}

/// A blocking test client that keeps its own terminal per attached block.
pub struct Client {
    stream: UnixStream,
    buf: Vec<u8>,
    id: ClientId,
    next_req: u64,
    terms: HashMap<BlockId, (Grid, AnsiParser)>,
    checkpoints: HashMap<BlockId, rusty_mux_server::checkpoint::Receiver>,
    /// Raw output received since attach, per block.
    output: HashMap<BlockId, Vec<u8>>,
    ready: HashMap<BlockId, bool>,
    /// Whether output is fed to the terminals. The flood test turns it off so
    /// it measures the server, not this debug-built client's parser.
    pub parse: bool,
    /// Where the Welcome said this client's event stream starts.
    pub cursor: Option<Cursor>,
    /// Every event received, with its number.
    pub events: Vec<(Option<u64>, Event)>,
}

impl Client {
    pub fn connect(path: &PathBuf) -> Self {
        Self::connect_with(path, Features::CHECKPOINT_V3, None)
    }

    pub fn connect_with(path: &PathBuf, features: Features, resume: Option<Cursor>) -> Self {
        let stream = UnixStream::connect(path).expect("connect");
        stream.set_read_timeout(Some(PATIENCE)).expect("timeout");
        let mut client = Self {
            stream,
            buf: Vec::new(),
            id: ClientId::from_uuid(rusty_uuid::Uuid::nil()),
            next_req: 1,
            terms: HashMap::new(),
            checkpoints: HashMap::new(),
            output: HashMap::new(),
            ready: HashMap::new(),
            parse: true,
            cursor: None,
            events: Vec::new(),
        };
        client.send(&Message::Hello { features, resume });
        match client.recv() {
            Message::Welcome {
                client: id, cursor, ..
            } => (client.id, client.cursor) = (id, cursor),
            other => panic!("expected Welcome, got {other:?}"),
        }
        client
    }

    pub fn send(&mut self, message: &Message) {
        self.stream
            .write_all(&encode(message).expect("encode"))
            .expect("write");
    }

    /// Reads one message, applying snapshots and output to its terminals.
    pub fn recv(&mut self) -> Message {
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
        if let Message::Event { seq, event } = message {
            self.events.push((*seq, event.clone()));
        }
        match message {
            Message::Checkpoint {
                block,
                offset,
                total,
                data,
            } => {
                if *offset == 0 {
                    self.checkpoints.insert(*block, Default::default());
                }
                self.checkpoints
                    .get_mut(block)
                    .expect("checkpoint started")
                    .push(*offset, *total, data)
                    .expect("valid checkpoint chunk");
                self.output.insert(*block, Vec::new());
                self.ready.insert(*block, false);
            }
            Message::Event {
                event: rusty_mux_core::Event::BlockResized { block, size },
                ..
            } => {
                if let Some((grid, _)) = self.terms.get_mut(block) {
                    grid.resize(size.cols().into(), size.rows().into());
                }
            }
            Message::Ready { block } => {
                let terminal = self
                    .checkpoints
                    .remove(block)
                    .expect("checkpoint")
                    .finish()
                    .expect("restore");
                self.terms.insert(*block, terminal);
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
    pub fn request(&mut self, make: impl FnOnce(u64) -> Message) -> Option<rusty_uuid::Uuid> {
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

    pub fn input(&mut self, block: BlockId, data: &[u8]) {
        let req = self.next_req;
        self.next_req += 1;
        self.send(&Message::Input {
            req,
            block,
            data: data.to_vec(),
        });
    }

    /// Reads until `block`'s output since attach contains `needle`.
    pub fn wait_for(&mut self, block: BlockId, needle: &str) {
        let deadline = Instant::now() + PATIENCE;
        let mut checked = 0usize;
        loop {
            let out = self
                .output
                .get(&block)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let start = checked
                .min(out.len())
                .saturating_sub(needle.len().saturating_sub(1));
            if out[start..]
                .windows(needle.len())
                .any(|w| w == needle.as_bytes())
            {
                return;
            }
            checked = out.len();
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {needle:?}"
            );
            self.recv();
        }
    }

    pub fn saw(&self, block: BlockId, needle: &str) -> bool {
        let out = self
            .output
            .get(&block)
            .map(Vec::as_slice)
            .unwrap_or_default();
        out.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    pub fn snapshot(&self, block: BlockId) -> Vec<u8> {
        self.terms[&block].0.encode_snapshot()
    }
}

/// Creates a session and a window from `client`.
pub fn window_for(client: &mut Client) -> WindowId {
    let session = SessionId::from_uuid(
        client
            .request(|req| Message::CreateSession {
                req,
                name: "s".into(),
            })
            .expect("id"),
    );
    WindowId::from_uuid(
        client
            .request(|req| Message::CreateWindow {
                req,
                session,
                name: "w".into(),
            })
            .expect("id"),
    )
}

/// Creates a block in `window`, neither attached nor leased.
pub fn new_block(client: &mut Client, window: WindowId) -> BlockId {
    let size = Size::new(80, 24).expect("size");
    BlockId::from_uuid(
        client
            .request(|req| Message::CreateBlock { req, window, size })
            .expect("id"),
    )
}

/// Creates a session, window and block from `client`, attached and leased.
pub fn block_for(client: &mut Client) -> BlockId {
    let window = window_for(client);
    let block = new_block(client, window);
    client.request(|req| Message::Attach { req, block });
    client.request(|req| Message::AcquireLease { req, block });
    block
}

/// Runs a command whose output differs from its echo, and waits for it.
pub fn run_and_wait(writer: &mut Client, block: BlockId, n: u32) -> String {
    let marker = format!("MARK{}", n * 2);
    writer.input(block, format!("echo MARK$(({n}*2))\r").as_bytes());
    writer.wait_for(block, &marker);
    marker
}

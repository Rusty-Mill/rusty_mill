//! Helpers shared by the server's integration tests: a real server on a
//! Unix socket and a blocking test client, on Linux and Windows.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[cfg(windows)]
use platform_windows::WindowsUnixStream as UnixStream;
use rusty_mux_core::{BlockId, ClientId, Event, SessionId, Size, WindowId};
use rusty_mux_proto::{Cursor, Features, Launch, Message, Takeover, decode, encode};
use rusty_mux_server::{Config, serve};
use rusty_term_core::{AnsiParser, Grid};
use rusty_tokio::io::UnixListener;
use rusty_tokio::{Builder, LocalOptions};

pub const PATIENCE: Duration = Duration::from_secs(20);

#[cfg(unix)]
pub fn connect(path: &Path) -> std::io::Result<UnixStream> {
    UnixStream::connect(path)
}

#[cfg(windows)]
pub fn connect(path: &Path) -> std::io::Result<UnixStream> {
    UnixStream::connect(path).map_err(|e| std::io::Error::other(format!("{e:?}")))
}

/// The shell each test block runs: `/bin/sh` in `/`, or `cmd.exe /Q` (no
/// command echo, so less re-rendered text crosses ConPTY) in the temp
/// directory.
pub fn shell() -> Config {
    if cfg!(windows) {
        Config {
            program: "cmd.exe".into(),
            args: vec!["/Q".into()],
            cwd: std::env::temp_dir().into_os_string(),
        }
    } else {
        Config {
            program: "/bin/sh".into(),
            args: Vec::new(),
            cwd: "/".into(),
        }
    }
}

/// A command whose output differs from its echo, and the output: seeing it
/// proves the shell ran the command. `cmd.exe` drops the `^` escape, while
/// the echoed line keeps it.
pub fn marker(n: u32) -> (String, String) {
    let input = if cfg!(windows) {
        format!("echo MARK^{}\r", n * 2)
    } else {
        format!("echo MARK$(({n}*2))\r")
    };
    (input, format!("MARK{}", n * 2))
}

/// Starts a server running [`shell`] in each block; returns its socket.
pub fn start_server() -> PathBuf {
    start_server_with(shell())
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
    /// The features the Welcome granted.
    pub features: Features,
    /// Every event received, with its number.
    pub events: Vec<(Option<u64>, Event)>,
    /// The takeover token the Welcome gave, with `TAKEOVER`.
    pub token: Option<rusty_uuid::Uuid>,
    /// Every `EchoAck` and `EchoMode`, each with how many bytes of its
    /// block's output since attach had arrived before it.
    pub echoes: Vec<(usize, Message)>,
}

impl Client {
    pub fn connect(path: &Path) -> Self {
        Self::connect_with(path, Features::CHECKPOINT_V6, None)
    }

    pub fn connect_with(path: &Path, features: Features, resume: Option<Cursor>) -> Self {
        Self::taking_over(path, features, resume, None)
    }

    /// Connects, presenting `takeover` in the Hello.
    pub fn taking_over(
        path: &Path,
        features: Features,
        resume: Option<Cursor>,
        takeover: Option<Takeover>,
    ) -> Self {
        let stream = connect(path).expect("connect");
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
            features: Features::NONE,
            events: Vec::new(),
            token: None,
            echoes: Vec::new(),
        };
        client.send(&Message::Hello {
            features,
            resume,
            takeover,
        });
        match client.recv() {
            Message::Welcome {
                client: id,
                cursor,
                features: granted,
                token,
            } => {
                (client.id, client.cursor, client.features) = (id, cursor, granted);
                client.token = token;
            }
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
            Message::EchoAck { block, .. } | Message::EchoMode { block, .. } => {
                let at = self.output.get(block).map_or(0, Vec::len);
                self.echoes.push((at, message.clone()));
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
                Message::Failed { req: r, reason, .. } if r == req => {
                    panic!("request {req} failed: {reason}")
                }
                _ => {}
            }
        }
    }

    /// Sends a request and returns every message answering it, ending with
    /// the one that completes it: `Done`, `Failed` or `Text`.
    pub fn answer(&mut self, make: impl FnOnce(u64) -> Message) -> Vec<Message> {
        let req = self.next_req;
        self.next_req += 1;
        self.send(&make(req));
        let mut answer = Vec::new();
        loop {
            let message = self.recv();
            let (r, last) = match &message {
                Message::Entry { req, .. } => (*req, false),
                Message::Done { req, .. }
                | Message::Failed { req, .. }
                | Message::Text { req, .. } => (*req, true),
                _ => continue,
            };
            if r != req {
                continue;
            }
            answer.push(message);
            if last {
                return answer;
            }
        }
    }

    pub fn id(&self) -> ClientId {
        self.id
    }

    /// Reads, discarding what arrives, until the server closes the
    /// connection; panics if it is still open after [`PATIENCE`].
    pub fn wait_closed(&mut self) {
        let mut chunk = [0u8; 65536];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => return,
                Ok(_) => {}
                // Windows reports a reset where Unix sees end of stream.
                Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset => return,
                Err(err) => panic!("connection still open: {err}"),
            }
        }
    }

    /// Sends input; its request number.
    pub fn input(&mut self, block: BlockId, data: &[u8]) -> u64 {
        let req = self.next_req;
        self.next_req += 1;
        self.send(&Message::Input {
            req,
            block,
            data: data.to_vec(),
        });
        req
    }

    /// `block`'s raw output since attach.
    pub fn output(&self, block: BlockId) -> &[u8] {
        self.output
            .get(&block)
            .map(Vec::as_slice)
            .unwrap_or_default()
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

    /// Reads until `block`'s output has `key` followed by a whole number,
    /// and returns the number.
    pub fn wait_for_number(&mut self, block: BlockId, key: &str) -> u32 {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let out = self
                .output
                .get(&block)
                .map(Vec::as_slice)
                .unwrap_or_default();
            if let Some(n) = number_after(out, key) {
                return n;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {key:?} and a number"
            );
            self.recv();
        }
    }

    /// Every line of `block`'s terminal as text.
    pub fn text(&self, block: BlockId) -> String {
        let grid = &self.terms[&block].0;
        (0..)
            .map_while(|n| grid.line_text(n))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Whether any checkpoint for `block` has arrived.
    pub fn has_checkpoint(&self, block: BlockId) -> bool {
        self.ready.contains_key(&block)
    }

    /// Reads until `block`'s checkpoint is in and ready.
    pub fn wait_ready(&mut self, block: BlockId) {
        while self.ready.get(&block) != Some(&true) {
            self.recv();
        }
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
            .request(|req| Message::CreateBlock {
                req,
                window,
                size,
                launch: Launch::default(),
            })
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
    let (input, expected) = marker(n);
    writer.input(block, input.as_bytes());
    writer.wait_for(block, &expected);
    expected
}

/// The first `key` in `out` followed by digits and then something else, so
/// the number is complete.
fn number_after(out: &[u8], key: &str) -> Option<u32> {
    let key = key.as_bytes();
    out.windows(key.len())
        .enumerate()
        .filter(|(_, w)| *w == key)
        .find_map(|(at, _)| {
            let tail = &out[at + key.len()..];
            let digits = tail.iter().take_while(|b| b.is_ascii_digit()).count();
            if digits == 0 || digits == tail.len() {
                return None;
            }
            std::str::from_utf8(&tail[..digits]).ok()?.parse().ok()
        })
}

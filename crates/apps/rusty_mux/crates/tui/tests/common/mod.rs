//! The scripted servers and the pty-hosted client that the lifecycle
//! tests share: on Linux the client runs on a pty, on Windows on our own
//! console host, which passes its VT bytes through, so the same byte
//! oracle holds on both (pack D209, D211).

#![allow(dead_code)]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use platform::process::{Child, Command, EnvSpec};
use platform::pty::PtyMaster;
use platform::term::WinSize;
use rusty_mux_core::{BlockId, ClientId, Event, Layout, SessionId, Size, WindowId};
use rusty_mux_proto::{Entry, Features, Message};
use rusty_mux_tui::{Conn, RESTORE};
use rusty_term_core::{AnsiParser, Grid};
use rusty_uuid::Uuid;

#[cfg(target_os = "linux")]
mod host {
    pub use platform_linux::{LinuxChild as Client, LinuxPty as Host, LinuxPtyMaster as Master};
    pub use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};

    /// Where the client runs.
    pub fn cwd() -> std::path::PathBuf {
        "/".into()
    }
}

#[cfg(windows)]
mod host {
    pub use platform_windows::{
        WindowsChild as Client, WindowsUnixListener as Listener, WindowsUnixStream as Stream,
    };
    pub use rusty_conhost::{OwnPty as Host, OwnPtyMaster as Master};

    /// Where the client runs.
    pub fn cwd() -> std::path::PathBuf {
        std::env::temp_dir()
    }
}

pub use host::Stream;
use host::{Host, Listener as UnixListener};

pub const BLOCK: &str = "6a1c0f3e-2b4d-4e5f-8a9b-0c1d2e3f4a5b";
pub const PATIENCE: Duration = Duration::from_secs(10);
/// Modes a block's program might leave set: alternate screen, hidden
/// cursor, bracketed paste, mouse reporting, and the win32-input-mode a
/// Windows pseudo console asks for.
pub const REMOTE: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[?2004h\x1b[?1000h\x1b[?9001hREMOTE";
/// [`REMOTE`] as the client passes it on, without win32-input-mode.
pub const SHOWN: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[?2004h\x1b[?1000hREMOTE";

/// How the scripted server ends the attach.
#[derive(Clone, Copy)]
pub enum End {
    /// Drops the connection.
    Eof,
    /// Reports the block closed.
    Closed,
    /// Waits for the client to leave.
    Open,
}

pub fn listen() -> (PathBuf, UnixListener) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("rusty-mux-tui-{}-{n}.sock", std::process::id()));
    let listener = UnixListener::bind(&path).expect("bind");
    (path, listener)
}

/// Accepts the next client and welcomes it with `features`. The client
/// waits for Welcome before sending more, so the stream holds nothing
/// unread.
pub fn welcome(listener: &UnixListener, features: Features) -> Stream {
    let (stream, _) = listener.accept().expect("accept");
    handshake(stream.try_clone().expect("clone"), features);
    stream
}

/// Takes the client's Hello and answers Welcome with `features`.
pub fn handshake(stream: Stream, features: Features) -> Conn<Stream> {
    let mut conn = Conn::new(stream);
    assert!(matches!(conn.recv().expect("hello"), Message::Hello { .. }));
    conn.send(&Message::Welcome {
        version: None,
        features,
        client: ClientId::from_uuid(Uuid::nil()),
        cursor: None,
        token: None,
    })
    .expect("welcome");
    conn
}

/// A server without layouts: it welcomes the client's control connection,
/// then attaches its block connection to [`BLOCK`] with an empty
/// checkpoint and [`REMOTE`] as output, and ends as `end` says once the
/// client asks for the lease.
pub fn scripted_server(lease: bool, end: End) -> (PathBuf, JoinHandle<()>) {
    let (path, listener) = listen();
    let server = std::thread::spawn(move || {
        let _control = welcome(&listener, Features::CHECKPOINT_V6);
        let pane = welcome(&listener, Features::CHECKPOINT_V6);
        serve_block(Conn::new(pane), lease, end);
    });
    (path, server)
}

/// Serves one block connection, which prints [`REMOTE`] once attached.
pub fn serve_block(mut conn: Conn<Stream>, lease: bool, end: End) {
    let mut attached = None;
    while let Ok(message) = conn.recv() {
        match message {
            Message::Attach { req, block } => {
                conn.send(&Message::Done { req, created: None })
                    .expect("done");
                attach(&mut conn, block, REMOTE);
                attached = Some(block);
            }
            Message::AcquireLease { req, block } => {
                let answer = match lease {
                    true => Message::Done { req, created: None },
                    false => Message::Failed {
                        req,
                        reason: "held by another client".into(),
                        code: None,
                    },
                };
                conn.send(&answer).expect("lease");
                if finish(&mut conn, attached.unwrap_or(block), end) {
                    return;
                }
            }
            Message::Resize { req, .. } => {
                conn.send(&Message::Done { req, created: None })
                    .expect("done");
            }
            _ => {}
        }
    }
}

/// The checkpoint of a blank 80x24 terminal, Ready, then `output`.
pub fn attach(conn: &mut Conn<Stream>, block: BlockId, output: &[u8]) {
    let data = rusty_term_core::checkpoint::encode(&Grid::new(80, 24), &AnsiParser::new())
        .expect("encode");
    let total = data.len() as u64;
    for (i, chunk) in data.chunks(65_536).enumerate() {
        conn.send(&Message::Checkpoint {
            block,
            offset: (i * 65_536) as u64,
            total,
            data: chunk.to_vec(),
        })
        .expect("checkpoint");
    }
    conn.send(&Message::Ready { block }).expect("ready");
    conn.send(&Message::Output {
        block,
        data: output.to_vec(),
    })
    .expect("output");
}

/// Ends the attach; true when the server should stop reading.
pub fn finish(conn: &mut Conn<Stream>, block: BlockId, end: End) -> bool {
    match end {
        End::Eof => true,
        End::Closed => {
            conn.send(&Message::Event {
                seq: None,
                event: Event::BlockClosed { block },
            })
            .expect("closed");
            false
        }
        End::Open => false,
    }
}

/// The client on a pty, with everything it writes collected.
pub struct Tui {
    pub master: Arc<host::Master>,
    pub child: host::Client,
    pub screen: Arc<Mutex<Vec<u8>>>,
}

impl Tui {
    pub fn start(socket: &Path) -> Self {
        Self::attach(socket, BLOCK)
    }

    pub fn attach(socket: &Path, block: &str) -> Self {
        Self::spawn(socket, block, EnvSpec::Inherit)
    }

    pub fn spawn(socket: &Path, block: &str, env: EnvSpec) -> Self {
        Self::run(&[socket.as_os_str(), block.as_ref()], env)
    }

    pub fn run(args: &[&OsStr], env: EnvSpec) -> Self {
        let cmd = args
            .iter()
            .fold(
                Command::new(env!("CARGO_BIN_EXE_rusty-mux"), host::cwd()),
                |cmd, arg| cmd.arg(arg),
            )
            .env(env);
        let (master, child) = Host
            .spawn_concrete(&cmd, WinSize { rows: 24, cols: 80 })
            .expect("spawn");
        let master = Arc::new(master);
        let screen = Arc::new(Mutex::new(Vec::new()));
        let (reader, sink) = (master.clone(), screen.clone());
        // Ends once the client exits: EIO on Linux, EOF from our host.
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                sink.lock().expect("lock").extend_from_slice(&buf[..n]);
            }
        });
        Self {
            master,
            child,
            screen,
        }
    }

    pub fn saw(&self, needle: &[u8]) -> bool {
        let screen = self.screen.lock().expect("lock");
        screen.windows(needle.len()).any(|w| w == needle)
    }

    /// How much the client has written so far.
    pub fn written(&self) -> usize {
        self.screen.lock().expect("lock").len()
    }

    /// What the client wrote after `from`.
    pub fn since(&self, from: usize) -> Vec<u8> {
        self.screen.lock().expect("lock")[from..].to_vec()
    }

    pub fn wait_for(&self, needle: &[u8]) {
        let deadline = Instant::now() + PATIENCE;
        while !self.saw(needle) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {needle:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits, bounded, for the client to exit on its own, and returns all
    /// it wrote.
    pub fn exit(self) -> Vec<u8> {
        self.exit_within(PATIENCE)
    }

    pub fn exit_within(mut self, patience: Duration) -> Vec<u8> {
        let deadline = Instant::now() + patience;
        while self.child.try_wait().expect("try_wait").is_none() {
            if Instant::now() >= deadline {
                let screen = self.screen.lock().expect("lock");
                let tail = &screen[screen.len().saturating_sub(2048)..];
                panic!(
                    "the client did not exit; it last wrote {:?}",
                    String::from_utf8_lossy(tail)
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Let the reader drain what the client wrote before it exited.
        std::thread::sleep(Duration::from_millis(200));
        self.screen.lock().expect("lock").clone()
    }
}

/// `RESTORE` comes after the program's modes, and `line` after both.
pub fn assert_restored(screen: &[u8], line: &str) {
    let find = |needle: &[u8]| screen.windows(needle.len()).rposition(|w| w == needle);
    let text = String::from_utf8_lossy(screen);
    let remote = find(SHOWN).unwrap_or_else(|| panic!("no remote output: {text:?}"));
    let restore = find(RESTORE).unwrap_or_else(|| panic!("terminal not reset: {text:?}"));
    let said = find(line.as_bytes()).unwrap_or_else(|| panic!("no {line:?}: {text:?}"));
    assert!(remote < restore && restore < said, "{text:?}");
}

pub fn keylog_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rusty-mux-keylog-{name}-{}", std::process::id()))
}

/// This process's environment with `RUSTY_MUX_KEYLOG` set to `log`.
pub fn keylog_env(log: &Path) -> EnvSpec {
    let mut env: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
    env.insert("RUSTY_MUX_KEYLOG".into(), log.as_os_str().to_owned());
    EnvSpec::Explicit(env)
}

pub const OTHER: &str = "0b5e2a7c-9d14-4f36-8e21-7a6c3b9d0e4f";
/// The left block's output: application cursor keys, bracketed paste and
/// its name.
pub const LEFT: &[u8] = b"\x1b[?1h\x1b[?2004hLEFT";

/// How a layout server behaves.
#[derive(Clone, Copy, Default)]
pub struct Script {
    /// The window holds [`BLOCK`] left of [`OTHER`]; else [`BLOCK`] alone.
    pub both: bool,
    /// A zoom is answered with the zoom and at once the unzoom.
    pub bounce: bool,
    /// The left block's lease is granted only once the right one asks.
    pub late_grant: bool,
    /// Once the left block's lease is granted, the control connection
    /// reports the window empty; later the block prints FINAL and closes.
    pub close: bool,
    /// Once the right block is attached, the control connection drops it
    /// from the layout; later the block prints BYE and closes.
    pub close_right: bool,
}

/// What the client asked of the blocks.
#[derive(Default)]
pub struct Log {
    pub resized: Vec<(BlockId, Size)>,
    pub released: Vec<BlockId>,
}

pub struct Shared {
    pub script: Script,
    pub window: WindowId,
    pub log: Mutex<Log>,
    /// Set once the right block asks for its lease.
    pub right_asked: (Mutex<bool>, Condvar),
    /// Events for the control connection to send.
    pub events: Mutex<Sender<Event>>,
    /// Set once the script closed the window's blocks: a List then lists
    /// none, as a server's does once a window's last block closed.
    pub emptied: AtomicBool,
}

impl Shared {
    pub fn layout(&self) -> Layout {
        match self.script.both {
            true => format!("cols50({BLOCK},{OTHER})").parse().expect("layout"),
            false => Layout::Block(id(BLOCK)),
        }
    }

    /// Has the control connection send `event`.
    pub fn tell(&self, event: Event) {
        let _ = self.events.lock().expect("lock").send(event);
    }

    pub fn released(&self) -> Vec<BlockId> {
        self.log.lock().expect("lock").released.clone()
    }
}

/// A server with layouts, serving `script`. Every block connection it
/// accepts is served on its own thread, for as long as the test runs.
pub fn layout_server(script: Script) -> (PathBuf, Arc<Shared>) {
    let (path, listener) = listen();
    let (events, queued) = mpsc::channel();
    let shared = Arc::new(Shared {
        script,
        window: WindowId::from_uuid(Uuid::nil()),
        log: Mutex::default(),
        right_asked: (Mutex::new(false), Condvar::new()),
        events: Mutex::new(events),
        emptied: AtomicBool::new(false),
    });
    let server = shared.clone();
    std::thread::spawn(move || {
        let features = Features::CHECKPOINT_V6
            .union(Features::INSPECT)
            .union(Features::LAYOUT);
        let control = welcome(&listener, features);
        let mut sender = Conn::new(control.try_clone().expect("clone"));
        let control_shared = server.clone();
        std::thread::spawn(move || serve_control(control, &control_shared));
        std::thread::spawn(move || {
            while let Ok(event) = queued.recv() {
                send_event(&mut sender, event);
            }
        });
        while let Ok((stream, _)) = listener.accept() {
            let block = server.clone();
            std::thread::spawn(move || {
                serve_layout_block(handshake(stream, Features::CHECKPOINT_V6), &block)
            });
        }
    });
    (path, shared)
}

pub fn layout_changed(window: WindowId, layout: Option<Layout>, zoomed: Option<BlockId>) -> Event {
    Event::LayoutChanged {
        window,
        layout,
        zoomed,
    }
}

pub fn send_event(conn: &mut Conn<Stream>, event: Event) {
    // The client may already have gone.
    let _ = conn.send(&Message::Event { seq: None, event });
}

/// Answers `list` with the window and its blocks, and zoom requests with
/// layout events.
pub fn serve_control(stream: Stream, shared: &Shared) {
    let mut conn = Conn::new(stream);
    let window = shared.window;
    while let Ok(message) = conn.recv() {
        match message {
            Message::List { req } => {
                let mut entries = vec![Entry::Window {
                    window,
                    session: SessionId::from_uuid(Uuid::nil()),
                    name: "main".into(),
                    layout: Some(shared.layout()),
                    zoomed: None,
                    focused: None,
                }];
                let blocks = match shared.emptied.load(Ordering::Acquire) {
                    true => Vec::new(),
                    false => shared.layout().blocks(),
                };
                for block in blocks {
                    entries.push(Entry::Block {
                        block,
                        window,
                        size: Size::new(80, 24).expect("size"),
                        holder: None,
                        title: String::new(),
                        name: String::new(),
                        owner: None,
                        input: rusty_mux_core::InputMode::Open,
                    });
                }
                for entry in entries {
                    conn.send(&Message::Entry { req, entry }).expect("entry");
                }
                conn.send(&Message::Done { req, created: None })
                    .expect("done");
            }
            Message::Zoom { req, block } => {
                conn.send(&Message::Done { req, created: None })
                    .expect("done");
                let layout = Some(shared.layout());
                send_event(
                    &mut conn,
                    layout_changed(window, layout.clone(), Some(block)),
                );
                if shared.script.bounce {
                    send_event(&mut conn, layout_changed(window, layout, None));
                }
            }
            _ => {}
        }
    }
}

/// After the control connection has dropped the block: its last output,
/// then its close, on its own connection.
pub fn last_words(conn: &mut Conn<Stream>, block: BlockId, data: &[u8]) {
    std::thread::sleep(Duration::from_millis(300));
    let _ = conn.send(&Message::Output {
        block,
        data: data.to_vec(),
    });
    send_event(conn, Event::BlockClosed { block });
}

/// Serves one block connection of a layout server.
pub fn serve_layout_block(mut conn: Conn<Stream>, shared: &Shared) {
    let script = shared.script;
    let (left, right) = (id(BLOCK), id(OTHER));
    while let Ok(message) = conn.recv() {
        match message {
            Message::Attach { req, block } => {
                conn.send(&Message::Done { req, created: None })
                    .expect("done");
                let output: &[u8] = if block == left { LEFT } else { b"RIGHT" };
                attach(&mut conn, block, output);
                if block == right && script.close_right {
                    let layout = Some(Layout::Block(left));
                    shared.tell(layout_changed(shared.window, layout, None));
                    last_words(&mut conn, block, b"BYE");
                }
            }
            Message::AcquireLease { req, block } => {
                if block == right {
                    let (asked, wake) = &shared.right_asked;
                    *asked.lock().expect("lock") = true;
                    wake.notify_all();
                }
                if block == left && script.late_grant {
                    let (asked, wake) = &shared.right_asked;
                    let guard = asked.lock().expect("lock");
                    let (_guard, _) = wake
                        .wait_timeout_while(guard, PATIENCE, |asked| !*asked)
                        .expect("wait");
                }
                conn.send(&Message::Done { req, created: None })
                    .expect("lease");
                if block == left && script.close {
                    shared.emptied.store(true, Ordering::Release);
                    shared.tell(layout_changed(shared.window, None, None));
                    last_words(&mut conn, block, b"FINAL");
                }
            }
            Message::ReleaseLease { req, block } => {
                shared.log.lock().expect("lock").released.push(block);
                let _ = conn.send(&Message::Done { req, created: None });
            }
            Message::Resize { req, block, size } => {
                shared.log.lock().expect("lock").resized.push((block, size));
                let _ = conn.send(&Message::Done { req, created: None });
            }
            _ => {}
        }
    }
}

pub fn id(text: &str) -> BlockId {
    BlockId::from_uuid(text.parse().expect("uuid"))
}

pub fn detach(tui: Tui) -> String {
    tui.master.write(b"\x1dd").expect("ctrl-] d");
    String::from_utf8_lossy(&tui.exit()).into_owned()
}

//! The client leaves the local terminal usable however the attach ends:
//! the block closing, the server going away (with no lease, so no key would
//! notice), or Ctrl-] d after the block's program set terminal modes; and
//! it draws a window of several blocks side by side. Each runs the real
//! binary on a pty against a scripted server.

#![cfg(target_os = "linux")]

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use platform::process::{Child, Command, EnvSpec};
use platform::pty::PtyMaster;
use platform::term::WinSize;
use platform_linux::{LinuxPty, LinuxPtyMaster};
use rusty_mux_core::{BlockId, ClientId, Event, Layout, SessionId, Size, WindowId};
use rusty_mux_proto::{Entry, Features, Message};
use rusty_mux_tui::{Conn, RESTORE};
use rusty_term_core::{AnsiParser, Grid};
use rusty_uuid::Uuid;

const BLOCK: &str = "6a1c0f3e-2b4d-4e5f-8a9b-0c1d2e3f4a5b";
const PATIENCE: Duration = Duration::from_secs(10);
/// Modes a block's program might leave set: alternate screen, hidden
/// cursor, bracketed paste, mouse reporting, and the win32-input-mode a
/// Windows pseudo console asks for.
const REMOTE: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[?2004h\x1b[?1000h\x1b[?9001hREMOTE";
/// [`REMOTE`] as the client passes it on, without win32-input-mode.
const SHOWN: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[?2004h\x1b[?1000hREMOTE";

/// How the scripted server ends the attach.
#[derive(Clone, Copy)]
enum End {
    /// Drops the connection.
    Eof,
    /// Reports the block closed.
    Closed,
    /// Waits for the client to leave.
    Open,
}

type Stream = std::os::unix::net::UnixStream;

fn listen() -> (PathBuf, UnixListener) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("rusty-mux-tui-{}-{n}.sock", std::process::id()));
    let listener = UnixListener::bind(&path).expect("bind");
    (path, listener)
}

/// Accepts the next client and welcomes it with `features`. The client
/// waits for Welcome before sending more, so the stream holds nothing
/// unread.
fn welcome(listener: &UnixListener, features: Features) -> Stream {
    let (stream, _) = listener.accept().expect("accept");
    handshake(stream.try_clone().expect("clone"), features);
    stream
}

/// Takes the client's Hello and answers Welcome with `features`.
fn handshake(stream: Stream, features: Features) -> Conn<Stream> {
    let mut conn = Conn::new(stream);
    assert!(matches!(conn.recv().expect("hello"), Message::Hello { .. }));
    conn.send(&Message::Welcome {
        features,
        client: ClientId::from_uuid(Uuid::nil()),
        cursor: None,
    })
    .expect("welcome");
    conn
}

/// A server without layouts: it welcomes the client's control connection,
/// then attaches its block connection to [`BLOCK`] with an empty
/// checkpoint and [`REMOTE`] as output, and ends as `end` says once the
/// client asks for the lease.
fn scripted_server(lease: bool, end: End) -> (PathBuf, JoinHandle<()>) {
    let (path, listener) = listen();
    let server = std::thread::spawn(move || {
        let _control = welcome(&listener, Features::CHECKPOINT_V5);
        let pane = welcome(&listener, Features::CHECKPOINT_V5);
        serve_block(Conn::new(pane), lease, end);
    });
    (path, server)
}

/// Serves one block connection, which prints [`REMOTE`] once attached.
fn serve_block(mut conn: Conn<Stream>, lease: bool, end: End) {
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
fn attach(conn: &mut Conn<Stream>, block: BlockId, output: &[u8]) {
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
fn finish(conn: &mut Conn<Stream>, block: BlockId, end: End) -> bool {
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
struct Tui {
    master: Arc<LinuxPtyMaster>,
    child: platform_linux::LinuxChild,
    screen: Arc<Mutex<Vec<u8>>>,
}

impl Tui {
    fn start(socket: &Path) -> Self {
        Self::attach(socket, BLOCK)
    }

    fn attach(socket: &Path, block: &str) -> Self {
        Self::spawn(socket, block, EnvSpec::Inherit)
    }

    fn spawn(socket: &Path, block: &str, env: EnvSpec) -> Self {
        let cmd = Command::new(env!("CARGO_BIN_EXE_rusty-mux"), "/")
            .arg(socket.as_os_str())
            .arg(block)
            .env(env);
        let (master, child) = LinuxPty
            .spawn_concrete(&cmd, WinSize { rows: 24, cols: 80 })
            .expect("spawn");
        let master = Arc::new(master);
        let screen = Arc::new(Mutex::new(Vec::new()));
        let (reader, sink) = (master.clone(), screen.clone());
        // Ends with EIO once the client exits and the slave closes.
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

    fn saw(&self, needle: &[u8]) -> bool {
        let screen = self.screen.lock().expect("lock");
        screen.windows(needle.len()).any(|w| w == needle)
    }

    /// How much the client has written so far.
    fn written(&self) -> usize {
        self.screen.lock().expect("lock").len()
    }

    /// What the client wrote after `from`.
    fn since(&self, from: usize) -> Vec<u8> {
        self.screen.lock().expect("lock")[from..].to_vec()
    }

    fn wait_for(&self, needle: &[u8]) {
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
    fn exit(mut self) -> Vec<u8> {
        let deadline = Instant::now() + PATIENCE;
        while self.child.try_wait().expect("try_wait").is_none() {
            assert!(Instant::now() < deadline, "the client did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
        // Let the reader drain what the client wrote before it exited.
        std::thread::sleep(Duration::from_millis(200));
        self.screen.lock().expect("lock").clone()
    }
}

/// `RESTORE` comes after the program's modes, and `line` after both.
fn assert_restored(screen: &[u8], line: &str) {
    let find = |needle: &[u8]| screen.windows(needle.len()).rposition(|w| w == needle);
    let text = String::from_utf8_lossy(screen);
    let remote = find(SHOWN).unwrap_or_else(|| panic!("no remote output: {text:?}"));
    let restore = find(RESTORE).unwrap_or_else(|| panic!("terminal not reset: {text:?}"));
    let said = find(line.as_bytes()).unwrap_or_else(|| panic!("no {line:?}: {text:?}"));
    assert!(remote < restore && restore < said, "{text:?}");
}

#[test]
fn a_view_only_client_exits_and_resets_when_the_server_goes_away() {
    let (socket, server) = scripted_server(false, End::Eof);
    let tui = Tui::start(&socket);
    let screen = tui.exit();
    server.join().expect("server");
    assert_restored(&screen, "lost the server");
}

#[test]
fn a_client_exits_and_resets_when_its_block_closes() {
    let (socket, server) = scripted_server(true, End::Closed);
    let tui = Tui::start(&socket);
    let screen = tui.exit();
    server.join().expect("server");
    assert_restored(&screen, "closed");
}

#[test]
fn detaching_after_the_program_set_modes_resets_the_terminal() {
    let (socket, server) = scripted_server(true, End::Open);
    let tui = Tui::start(&socket);
    tui.wait_for(b"REMOTE");
    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let screen = tui.exit();
    server.join().expect("server");
    assert_restored(&screen, "detached from block");
}

/// The local terminal must keep sending plain keys, or Ctrl-] arrives
/// win32-encoded and passes to the block: the client drops the program's
/// request for the mode.
#[test]
fn win32_input_mode_never_reaches_the_local_terminal() {
    let (socket, server) = scripted_server(true, End::Open);
    let tui = Tui::start(&socket);
    tui.wait_for(SHOWN);
    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let screen = tui.exit();
    server.join().expect("server");
    let asked = screen.windows(8).any(|w| w == b"\x1b[?9001h");
    assert!(!asked, "{:?}", String::from_utf8_lossy(&screen));
}

#[test]
fn the_keylog_records_each_keyboard_read_in_hex() {
    let log = keylog_path("new");
    let _ = std::fs::remove_file(&log);
    let (socket, server) = scripted_server(true, End::Open);
    let tui = Tui::spawn(&socket, BLOCK, keylog_env(&log));
    tui.wait_for(b"REMOTE");
    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let screen = tui.exit();
    server.join().expect("server");
    let logged = std::fs::read_to_string(&log).expect("keylog");
    assert_eq!(logged, "1d 64\n", "{logged:?}");
    let mode = std::os::unix::fs::PermissionsExt::mode(
        &std::fs::metadata(&log).expect("stat").permissions(),
    );
    assert_eq!(mode & 0o777, 0o600, "{mode:o}");
    let warned = String::from_utf8_lossy(&screen).contains("passwords included");
    assert!(warned, "{:?}", String::from_utf8_lossy(&screen));
    std::fs::remove_file(&log).expect("cleanup");
}

/// The keys must never land in a file another user may already read.
#[test]
fn the_keylog_refuses_an_existing_file() {
    let log = keylog_path("existing");
    std::fs::write(&log, "kept\n").expect("existing log");
    let (socket, _server) = scripted_server(true, End::Open);
    let screen = Tui::spawn(&socket, BLOCK, keylog_env(&log)).exit();
    let text = String::from_utf8_lossy(&screen);
    assert!(text.contains("RUSTY_MUX_KEYLOG"), "{text:?}");
    assert_eq!(std::fs::read_to_string(&log).expect("log"), "kept\n");
    std::fs::remove_file(&log).expect("cleanup");
}

fn keylog_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rusty-mux-keylog-{name}-{}", std::process::id()))
}

/// This process's environment with `RUSTY_MUX_KEYLOG` set to `log`.
fn keylog_env(log: &Path) -> EnvSpec {
    let mut env: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
    env.insert("RUSTY_MUX_KEYLOG".into(), log.as_os_str().to_owned());
    EnvSpec::Explicit(env)
}

const OTHER: &str = "0b5e2a7c-9d14-4f36-8e21-7a6c3b9d0e4f";
/// The left block's output: application cursor keys, bracketed paste and
/// its name.
const LEFT: &[u8] = b"\x1b[?1h\x1b[?2004hLEFT";

/// How a layout server behaves.
#[derive(Clone, Copy, Default)]
struct Script {
    /// The window holds [`BLOCK`] left of [`OTHER`]; else [`BLOCK`] alone.
    both: bool,
    /// A zoom is answered with the zoom and at once the unzoom.
    bounce: bool,
    /// The left block's lease is granted only once the right one asks.
    late_grant: bool,
    /// Once the left block's lease is granted, the control connection
    /// reports the window empty; later the block prints FINAL and closes.
    close: bool,
    /// Once the right block is attached, the control connection drops it
    /// from the layout; later the block prints BYE and closes.
    close_right: bool,
}

/// What the client asked of the blocks.
#[derive(Default)]
struct Log {
    resized: Vec<(BlockId, Size)>,
    released: Vec<BlockId>,
}

struct Shared {
    script: Script,
    window: WindowId,
    log: Mutex<Log>,
    /// Set once the right block asks for its lease.
    right_asked: (Mutex<bool>, Condvar),
    /// Events for the control connection to send.
    events: Mutex<Sender<Event>>,
}

impl Shared {
    fn layout(&self) -> Layout {
        match self.script.both {
            true => format!("cols50({BLOCK},{OTHER})").parse().expect("layout"),
            false => Layout::Block(id(BLOCK)),
        }
    }

    /// Has the control connection send `event`.
    fn tell(&self, event: Event) {
        let _ = self.events.lock().expect("lock").send(event);
    }

    fn released(&self) -> Vec<BlockId> {
        self.log.lock().expect("lock").released.clone()
    }
}

/// A server with layouts, serving `script`. Every block connection it
/// accepts is served on its own thread, for as long as the test runs.
fn layout_server(script: Script) -> (PathBuf, Arc<Shared>) {
    let (path, listener) = listen();
    let (events, queued) = mpsc::channel();
    let shared = Arc::new(Shared {
        script,
        window: WindowId::from_uuid(Uuid::nil()),
        log: Mutex::default(),
        right_asked: (Mutex::new(false), Condvar::new()),
        events: Mutex::new(events),
    });
    let server = shared.clone();
    std::thread::spawn(move || {
        let features = Features::CHECKPOINT_V5
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
                serve_layout_block(handshake(stream, Features::CHECKPOINT_V5), &block)
            });
        }
    });
    (path, shared)
}

fn layout_changed(window: WindowId, layout: Option<Layout>, zoomed: Option<BlockId>) -> Event {
    Event::LayoutChanged {
        window,
        layout,
        zoomed,
    }
}

fn send_event(conn: &mut Conn<Stream>, event: Event) {
    // The client may already have gone.
    let _ = conn.send(&Message::Event { seq: None, event });
}

/// Answers `list` with the window and its blocks, and zoom requests with
/// layout events.
fn serve_control(stream: Stream, shared: &Shared) {
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
                }];
                for block in shared.layout().blocks() {
                    entries.push(Entry::Block {
                        block,
                        window,
                        size: Size::new(80, 24).expect("size"),
                        holder: None,
                        title: String::new(),
                        name: String::new(),
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
fn last_words(conn: &mut Conn<Stream>, block: BlockId, data: &[u8]) {
    std::thread::sleep(Duration::from_millis(300));
    let _ = conn.send(&Message::Output {
        block,
        data: data.to_vec(),
    });
    send_event(conn, Event::BlockClosed { block });
}

/// Serves one block connection of a layout server.
fn serve_layout_block(mut conn: Conn<Stream>, shared: &Shared) {
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

fn id(text: &str) -> BlockId {
    BlockId::from_uuid(text.parse().expect("uuid"))
}

fn detach(tui: Tui) -> String {
    tui.master.write(b"\x1dd").expect("ctrl-] d");
    String::from_utf8_lossy(&tui.exit()).into_owned()
}

#[test]
fn a_window_of_two_blocks_is_drawn_side_by_side() {
    let (socket, server) = layout_server(Script {
        both: true,
        ..Script::default()
    });
    let tui = Tui::start(&socket);
    tui.wait_for(b"LEFT");
    tui.wait_for(b"RIGHT");
    tui.wait_for("│".as_bytes());
    let text = detach(tui);
    // Only the focused block, which holds the lease, fits its half.
    let resized = server.log.lock().expect("lock").resized.clone();
    assert_eq!(resized, vec![(id(BLOCK), Size::new(40, 24).expect("size"))]);
    let find = |needle: &str| text.rfind(needle);
    let alternate = find("\x1b[?1049h").unwrap_or_else(|| panic!("no alternate screen: {text:?}"));
    let restore = find(std::str::from_utf8(RESTORE).expect("utf-8"))
        .unwrap_or_else(|| panic!("terminal not reset: {text:?}"));
    let said = find("detached from block").unwrap_or_else(|| panic!("no detach: {text:?}"));
    assert!(alternate < restore && restore < said, "{text:?}");
}

#[test]
fn a_lease_granted_after_the_focus_moved_is_given_back() {
    let (socket, server) = layout_server(Script {
        both: true,
        late_grant: true,
        ..Script::default()
    });
    let tui = Tui::start(&socket);
    tui.wait_for(b"RIGHT");
    // Focus the right block while the left one's lease is still asked for.
    tui.master.write(b"\x1dl").expect("ctrl-] l");
    let deadline = Instant::now() + PATIENCE;
    while !server.released().contains(&id(BLOCK)) {
        assert!(Instant::now() < deadline, "the left lease was kept");
        std::thread::sleep(Duration::from_millis(20));
    }
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
}

#[test]
fn a_block_hidden_and_shown_again_at_once_keeps_the_client() {
    let (socket, _server) = layout_server(Script {
        both: true,
        bounce: true,
        ..Script::default()
    });
    let tui = Tui::start(&socket);
    tui.wait_for(b"RIGHT");
    // The right block's first connection ends after its second opens.
    let from = tui.written();
    tui.master.write(b"\x1dz").expect("ctrl-] z");
    let deadline = Instant::now() + PATIENCE;
    while tui.since(from).windows(5).filter(|w| w == b"RIGHT").count() == 0 {
        assert!(
            Instant::now() < deadline,
            "the right block was not shown again"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(300));
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
}

#[test]
fn a_closed_window_shows_its_blocks_last_output_first() {
    // Passed through alone, and composed beside a block that never closes.
    for both in [false, true] {
        let (socket, _server) = layout_server(Script {
            both,
            close: true,
            ..Script::default()
        });
        let tui = Tui::start(&socket);
        let text = String::from_utf8_lossy(&tui.exit()).into_owned();
        let restore = text
            .rfind(std::str::from_utf8(RESTORE).expect("utf-8"))
            .unwrap_or_else(|| panic!("terminal not reset: {text:?}"));
        let last = text
            .rfind("FINAL")
            .unwrap_or_else(|| panic!("no last output ({both}): {text:?}"));
        let said = text
            .rfind("closed")
            .unwrap_or_else(|| panic!("no close: {text:?}"));
        assert!(last < restore && restore < said, "{both}: {text:?}");
    }
}

#[test]
fn a_block_dropped_from_the_layout_shows_its_last_output_first() {
    let (socket, _server) = layout_server(Script {
        both: true,
        close_right: true,
        ..Script::default()
    });
    let tui = Tui::start(&socket);
    // The layout drops the right block before its last output arrives.
    tui.wait_for(b"RIGHTBYE");
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
}

#[test]
fn a_block_passed_through_again_gets_its_modes_back() {
    let (socket, _server) = layout_server(Script {
        both: true,
        ..Script::default()
    });
    let tui = Tui::start(&socket);
    tui.wait_for(b"RIGHT");
    // Composing reset the left block's modes; zooming it passes it again.
    let from = tui.written();
    tui.master.write(b"\x1dz").expect("ctrl-] z");
    let deadline = Instant::now() + PATIENCE;
    let passed = loop {
        let since = String::from_utf8_lossy(&tui.since(from)).into_owned();
        if let Some(at) = since.find("\x1b[1;24r") {
            break since[at..].to_owned();
        }
        assert!(Instant::now() < deadline, "not passed through: {since:?}");
        std::thread::sleep(Duration::from_millis(20));
    };
    detach(tui);
    assert!(
        passed.contains("\x1b[?1h") && passed.contains("\x1b[?2004h"),
        "{passed:?}"
    );
}

#[test]
fn a_server_offering_only_v3_checkpoints_is_refused() {
    // An older server's RTCP v3 has no colour provenance; the client
    // refuses it rather than restore its default colours baked in.
    let (socket, listener) = listen();
    let server = std::thread::spawn(move || welcome(&listener, Features::CHECKPOINT_V3));
    let tui = Tui::start(&socket);
    let screen = tui.exit();
    drop(server.join().expect("server"));
    let text = String::from_utf8_lossy(&screen);
    assert!(text.contains("cannot send checkpoints"), "{text:?}");
}

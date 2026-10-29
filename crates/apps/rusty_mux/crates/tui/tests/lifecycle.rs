//! The client leaves the local terminal usable however the attach ends:
//! the block closing, the server going away (with no lease, so no key would
//! notice), or Ctrl-] d after the block's program set terminal modes; and
//! it draws a window of several blocks side by side. Each runs the real
//! binary on a pty against a scripted server; the reconnect tests, at the
//! end, run it against a real one through a stand-in `ssh`.

#![cfg(target_os = "linux")]

use std::ffi::OsStr;
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
        token: None,
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
        let _control = welcome(&listener, Features::CHECKPOINT_V6);
        let pane = welcome(&listener, Features::CHECKPOINT_V6);
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
        Self::run(&[socket.as_os_str(), block.as_ref()], env)
    }

    fn run(args: &[&OsStr], env: EnvSpec) -> Self {
        let cmd = args
            .iter()
            .fold(
                Command::new(env!("CARGO_BIN_EXE_rusty-mux"), "/"),
                |cmd, arg| cmd.arg(arg),
            )
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
    fn exit(self) -> Vec<u8> {
        self.exit_within(PATIENCE)
    }

    fn exit_within(mut self, patience: Duration) -> Vec<u8> {
        let deadline = Instant::now() + patience;
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

/// `--ssh` runs the forward, connects through it, and on detach stops it
/// and removes its socket's directory. The stand-in `ssh` records its
/// arguments and pid, then "forwards" by linking the local path to the
/// server's socket.
#[test]
fn an_ssh_attach_goes_through_the_forward_and_cleans_up() {
    let work = std::env::temp_dir().join(format!("rusty-mux-fake-ssh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir(&work).expect("work dir");
    let fake = work.join("ssh");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > {w}/args\necho $$ > {w}/pid\n\
         while [ \"$1\" != -L ]; do shift; done\n\
         ln -s \"${{2#*:}}\" \"${{2%%:*}}\"\nexec sleep 600\n",
        w = work.display()
    );
    std::fs::write(&fake, script).expect("fake ssh");
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .expect("chmod");
    let mut env: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
    env.insert("RUSTY_MUX_SSH".into(), fake.into_os_string());

    let (socket, server) = scripted_server(true, End::Open);
    let args = [
        OsStr::new("--ssh"),
        OsStr::new("box"),
        socket.as_os_str(),
        BLOCK.as_ref(),
    ];
    let tui = Tui::run(&args, EnvSpec::Explicit(env));
    tui.wait_for(b"REMOTE");
    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let screen = tui.exit();
    server.join().expect("server");
    assert_restored(&screen, "detached from block");

    let args = std::fs::read_to_string(work.join("args")).expect("args");
    let args: Vec<&str> = args.lines().collect();
    assert_eq!(args[args.len() - 2..], ["--", "box"], "{args:?}");
    let forward = args.iter().find(|a| a.contains(':')).expect("-L argument");
    let (local, remote) = forward.split_once(':').expect("local:remote");
    assert_eq!(Path::new(remote), socket);
    let dir = Path::new(local).parent().expect("socket dir");
    assert!(!dir.exists(), "{} was left behind", dir.display());
    let pid = std::fs::read_to_string(work.join("pid")).expect("pid");
    let proc = Path::new("/proc").join(pid.trim());
    assert!(!proc.exists(), "ssh {} still runs", pid.trim());
    std::fs::remove_dir_all(&work).expect("cleanup");
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

// Reconnecting after a lost path, against a real server reached through a
// stand-in `ssh` that forwards like OpenSSH: it relays each connection to
// the local socket on to the server's, so stopping it freezes the path as
// a dead network does, and killing it ends every connection.

/// A real server on a new socket, with `/bin/sh` for each block.
fn real_server() -> PathBuf {
    use rusty_mux_server::{Config, serve};
    use rusty_tokio::io::UnixListener;
    use rusty_tokio::{Builder, LocalOptions};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "rusty-mux-tui-real-{}-{n}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let (ready_tx, ready_rx) = mpsc::channel();
    let bind = path.clone();
    std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime");
        runtime.block_on(async move {
            let listener = UnixListener::bind(&bind).expect("bind");
            ready_tx.send(()).expect("ready");
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

/// A work directory holding a stand-in `ssh` that logs `pid args...` to
/// `log` and relays the `-L` local socket to the remote one. Its `n`th
/// start (from 0) follows line `n` of `plan`, if there is one:
///
/// - `fail`: exits at once, as a refused login does.
/// - `hold`: accepts connections but passes nothing on, either way.
/// - `stall C F`: relays, but passes only the first `F` of the server's
///   messages on its `C`th connection (from 0), and holds the rest.
///
/// SIGUSR1 cuts every connection on the client's side only, so the client
/// sees them end while the server keeps its clients.
fn relaying_ssh(name: &str, plan: &[&str]) -> PathBuf {
    let work = std::env::temp_dir().join(format!("rusty-mux-relay-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir(&work).expect("work dir");
    std::fs::write(work.join("plan"), plan.join("\n")).expect("plan");
    let script = format!(
        r#"#!/usr/bin/env python3
import os, signal, socket, sys, threading
args = sys.argv[1:]
with open("{log}", "a") as log:
    log.write(" ".join([str(os.getpid())] + args) + "\n")
with open("{log}") as log:
    start = len(log.read().splitlines()) - 1
with open("{plan}") as plan:
    plans = plan.read().splitlines()
mode = (plans[start] if start < len(plans) else "relay").split()
if mode[0] == "fail":
    sys.exit(255)
stall = (int(mode[1]), int(mode[2])) if mode[0] == "stall" else None
local, remote = args[args.index("-L") + 1].split(":", 1)
listener = socket.socket(socket.AF_UNIX)
listener.bind(local)
listener.listen()
nears = []
cut = False
def on_cut(*_):
    global cut
    cut = True
    for near in nears:
        try:
            near.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
signal.signal(signal.SIGUSR1, on_cut)
def pump(a, b, frames=None):
    held = b""
    try:
        while True:
            data = a.recv(65536)
            if not data:
                break
            if frames is None:
                b.sendall(data)
                continue
            held += data
            while frames > 0 and len(held) >= 4:
                end = 4 + int.from_bytes(held[:4], "big")
                if len(held) < end:
                    break
                b.sendall(held[:end])
                held = held[end:]
                frames -= 1
    except OSError:
        pass
    for end in (a, b):
        if cut and end not in nears:
            continue
        try:
            end.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
held = []
while True:
    near, _ = listener.accept()
    nears.append(near)
    if mode[0] == "hold":
        held.append(near)
        continue
    far = socket.socket(socket.AF_UNIX)
    far.connect(remote)
    frames = stall[1] if stall and stall[0] == len(nears) - 1 else None
    threading.Thread(target=pump, args=(near, far), daemon=True).start()
    threading.Thread(target=pump, args=(far, near, frames), daemon=True).start()
"#,
        log = work.join("log").display(),
        plan = work.join("plan").display(),
    );
    let ssh = work.join("ssh");
    std::fs::write(&ssh, script).expect("stand-in ssh");
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .expect("chmod");
    work
}

/// Each start of the stand-in so far: its pid and arguments.
fn logins(work: &Path) -> Vec<(u32, String)> {
    let log = std::fs::read_to_string(work.join("log")).unwrap_or_default();
    log.lines()
        .filter_map(|line| {
            let (pid, args) = line.split_once(' ')?;
            Some((pid.parse().ok()?, args.to_owned()))
        })
        .collect()
}

/// Waits, up to `patience`, for the stand-in's `n`th start.
fn wait_logins(work: &Path, n: usize, patience: Duration) -> Vec<(u32, String)> {
    let deadline = Instant::now() + patience;
    loop {
        let seen = logins(work);
        if seen.len() >= n {
            return seen;
        }
        assert!(
            Instant::now() < deadline,
            "only {} logins: {seen:?}",
            seen.len()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn signal(pid: u32, signal: &str) {
    let status = std::process::Command::new("kill")
        .args([signal, &pid.to_string()])
        .status()
        .expect("kill");
    assert!(status.success(), "kill {signal} {pid}");
}

fn running(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

/// The client through the stand-in in `work` to the server at `socket`,
/// on a new block.
fn ssh_client(work: &Path, socket: &Path) -> Tui {
    let mut env: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
    env.insert("RUSTY_MUX_SSH".into(), work.join("ssh").into_os_string());
    let args = [OsStr::new("--ssh"), OsStr::new("box"), socket.as_os_str()];
    Tui::run(&args, EnvSpec::Explicit(env))
}

/// Types `command` until `needle` shows: keys typed before the client
/// holds the block's lease are not the block's.
fn type_until(tui: &Tui, command: &[u8], needle: &[u8]) {
    let deadline = Instant::now() + PATIENCE;
    while !tui.saw(needle) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {needle:?}"
        );
        tui.master.write(command).expect("type");
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// A path that goes silent, as on a network change, is noticed by the
/// heartbeat and brought back with a second login that may not prompt;
/// the client takes its clients over, so the block's screen comes back
/// and typing reaches it without asking for the lease again. The frozen
/// first `ssh` is stopped only after the takeover, and nothing is left
/// once the client detaches.
#[test]
fn a_frozen_path_is_brought_back_with_the_screen_and_the_lease() {
    let socket = real_server();
    let work = relaying_ssh("frozen", &[]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    let first = wait_logins(&work, 1, PATIENCE)[0].0;
    signal(first, "-STOP");
    let frozen = tui.written();

    // Noticed after 15 s without a sign of life, then logged in again.
    let seen = wait_logins(&work, 2, Duration::from_secs(30));
    assert!(!seen[0].1.contains("BatchMode"), "{seen:?}");
    assert!(seen[1].1.contains("BatchMode=yes"), "{seen:?}");
    // The takeover sends the block's screen again; keys typed before it
    // are dropped, since the path is still down.
    let deadline = Instant::now() + PATIENCE;
    while !tui.since(frozen).windows(5).any(|w| w == b"one-1") {
        assert!(Instant::now() < deadline, "the screen did not come back");
        std::thread::sleep(Duration::from_millis(20));
    }
    // Typed once: the lease came along, so the first keys count.
    let from = tui.written();
    tui.master.write(b"echo two-$((1+1))\r").expect("type");
    tui.wait_for(b"two-2");
    let echoed = tui.since(from);
    let twice = echoed.windows(5).filter(|w| *w == b"two-2").count();
    assert_eq!(twice, 1, "{:?}", String::from_utf8_lossy(&echoed));
    assert!(!running(first), "the frozen ssh outlived the takeover");

    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let screen = tui.exit();
    let text = String::from_utf8_lossy(&screen);
    assert!(text.contains("detached from block"), "{text:?}");
    assert!(!text.contains("path to the server was down"), "{text:?}");
    assert!(!running(seen[1].0), "the second ssh outlived the client");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// Reconnecting only ever continues the clients the server still has: one
/// the server let go meanwhile (here, ended by another client) is not
/// replaced by a new one, and the client stops and says so.
#[test]
fn a_client_the_server_let_go_is_not_brought_back() {
    let socket = real_server();
    let work = relaying_ssh("dropped", &[]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    disconnect_everyone(&socket);

    let screen = tui.exit();
    let text = String::from_utf8_lossy(&screen);
    assert!(text.contains("the server let this client go"), "{text:?}");
    let seen = logins(&work);
    assert_eq!(seen.len(), 2, "one login to try again: {seen:?}");
    assert!(seen.iter().all(|(pid, _)| !running(*pid)), "{seen:?}");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// Ends every other client of the server at `socket`, as Disconnect does.
fn disconnect_everyone(socket: &Path) {
    let mut conn = Conn::new(Stream::connect(socket).expect("connect"));
    conn.send(&Message::Hello {
        features: Features::CHECKPOINT_V6
            .union(Features::INSPECT)
            .union(Features::LABELS),
        resume: None,
        takeover: None,
    })
    .expect("hello");
    let me = match conn.recv().expect("welcome") {
        Message::Welcome { client, .. } => client,
        other => panic!("expected Welcome, got {other:?}"),
    };
    conn.send(&Message::List { req: 1 }).expect("list");
    let mut others = Vec::new();
    loop {
        match conn.recv().expect("list answer") {
            Message::Entry {
                entry: Entry::Client { client, .. },
                ..
            } if client != me => others.push(client),
            Message::Done { req: 1, .. } => break,
            _ => {}
        }
    }
    assert!(!others.is_empty(), "no clients to end");
    for (req, client) in (2..).zip(others) {
        conn.send(&Message::Disconnect { req, client })
            .expect("disconnect");
        loop {
            match conn.recv().expect("disconnect answer") {
                Message::Done { req: r, .. } if r == req => break,
                Message::Failed { req: r, reason, .. } if r == req => panic!("{reason}"),
                _ => {}
            }
        }
    }
}

/// What the client's output leaves on an 80x24 terminal, row by row.
fn screen_text(output: &[u8]) -> String {
    let mut grid = Grid::new(80, 24);
    AnsiParser::new().advance(&mut grid, output);
    (0..grid.rows)
        .map(|y| grid.row_cells(y).map(|cell| cell.ch).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Loses the path at once: the first stand-in cuts the client's side of
/// every connection, and the server keeps the clients.
fn cut(work: &Path) {
    signal(wait_logins(work, 1, PATIENCE)[0].0, "-USR1");
}

/// A fresh path that comes up but never answers is given up on within
/// the handshake's bound, its `ssh` stopped, and tried again; the next
/// one brings the block back with its lease.
#[test]
fn a_fresh_path_that_never_answers_is_given_up_and_tried_again() {
    let socket = real_server();
    let work = relaying_ssh("hold", &["relay", "hold"]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    cut(&work);
    let held = wait_logins(&work, 2, PATIENCE)[1].0;
    let from = tui.written();
    // 10 s for the answers, then 2 s before trying again.
    let seen = wait_logins(&work, 3, Duration::from_secs(20));
    assert!(!running(held), "the silent ssh was kept");
    let deadline = Instant::now() + PATIENCE;
    while !tui.since(from).windows(5).any(|w| w == b"one-1") {
        assert!(Instant::now() < deadline, "the screen did not come back");
        std::thread::sleep(Duration::from_millis(20));
    }
    let from = tui.written();
    tui.master.write(b"echo two-$((1+1))\r").expect("type");
    tui.wait_for(b"two-2");
    assert!(tui.since(from).windows(5).any(|w| w == b"two-2"));
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
    assert!(seen.iter().all(|(pid, _)| !running(*pid)), "{seen:?}");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// Once the control connection's client is taken over, a fresh path that
/// stops answering (here, before the layout) cannot be given up on and
/// tried again, since hanging it up ends that client: the client stops
/// within the handshake's bound and says why, and both `ssh`s go.
#[test]
fn a_takeover_that_stalls_midway_stops_the_client_in_time() {
    let socket = real_server();
    let work = relaying_ssh("stall-list", &["relay", "stall 0 1"]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    cut(&work);
    wait_logins(&work, 2, PATIENCE);
    let screen = tui.exit_within(Duration::from_secs(20));
    let text = String::from_utf8_lossy(&screen);
    assert!(text.contains("reconnecting: "), "{text:?}");
    let seen = logins(&work);
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(seen.iter().all(|(pid, _)| !running(*pid)), "{seen:?}");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// Ctrl-] d detaches at once while a block's takeover waits on an answer
/// that never comes, well before the handshake's bound, and both `ssh`s
/// go with the client.
#[test]
fn detaching_while_a_takeover_stalls_is_prompt() {
    let socket = real_server();
    let work = relaying_ssh("stall-pane", &["relay", "stall 1 0"]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    cut(&work);
    wait_logins(&work, 2, PATIENCE);
    // Past the login and the control connection's takeover.
    std::thread::sleep(Duration::from_secs(1));
    let asked = Instant::now();
    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let screen = tui.exit_within(Duration::from_secs(3));
    assert!(asked.elapsed() < Duration::from_secs(3));
    let text = String::from_utf8_lossy(&screen);
    assert!(text.contains("detached from block"), "{text:?}");
    let seen = logins(&work);
    assert!(seen.iter().all(|(pid, _)| !running(*pid)), "{seen:?}");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// A split window stays on screen while the path is down and a login
/// fails, and both blocks are drawn again once it is back.
#[test]
fn a_split_window_stays_on_screen_while_the_path_is_down() {
    let socket = real_server();
    let work = relaying_ssh("split", &["relay", "fail"]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    tui.master.write(b"\x1d|").expect("ctrl-] |");
    type_until(&tui, b"echo two-$((1+1))\r", b"two-2");
    let shown = |tui: &Tui| {
        let text = screen_text(&tui.since(0));
        text.contains("one-1") && text.contains("two-2")
    };
    let deadline = Instant::now() + PATIENCE;
    while !shown(&tui) {
        assert!(Instant::now() < deadline, "the split did not show both");
        std::thread::sleep(Duration::from_millis(20));
    }
    cut(&work);
    // The failed login, then 2 s before the next.
    wait_logins(&work, 2, PATIENCE);
    std::thread::sleep(Duration::from_millis(500));
    let text = screen_text(&tui.since(0));
    assert!(text.contains("one-1") && text.contains("two-2"), "{text}");

    wait_logins(&work, 3, PATIENCE);
    let from = tui.written();
    let deadline = Instant::now() + PATIENCE;
    loop {
        let again = String::from_utf8_lossy(&tui.since(from)).into_owned();
        if again.contains("one-1") && again.contains("two-2") {
            break;
        }
        assert!(Instant::now() < deadline, "not drawn again: {again:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(shown(&tui), "{}", screen_text(&tui.since(0)));
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
    let seen = logins(&work);
    assert!(seen.iter().all(|(pid, _)| !running(*pid)), "{seen:?}");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

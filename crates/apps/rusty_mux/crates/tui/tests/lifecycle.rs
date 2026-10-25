//! The client leaves the local terminal usable however the attach ends:
//! the block closing, the server going away (with no lease, so no key would
//! notice), or Ctrl-] after the block's program set terminal modes. Each
//! runs the real binary on a pty against a scripted server.

#![cfg(target_os = "linux")]

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use platform::process::{Child, Command};
use platform::pty::PtyMaster;
use platform::term::WinSize;
use platform_linux::{LinuxPty, LinuxPtyMaster};
use rusty_mux_core::{BlockId, ClientId, Event};
use rusty_mux_proto::{Features, Message};
use rusty_mux_tui::{Conn, RESTORE};
use rusty_term_core::{AnsiParser, Grid};
use rusty_uuid::Uuid;

const BLOCK: &str = "6a1c0f3e-2b4d-4e5f-8a9b-0c1d2e3f4a5b";
const PATIENCE: Duration = Duration::from_secs(10);
/// Modes a block's program might leave set: alternate screen, hidden
/// cursor, bracketed paste and mouse reporting.
const REMOTE: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[?2004h\x1b[?1000hREMOTE";

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

/// A server that welcomes one client, attaches it to [`BLOCK`] with an
/// empty checkpoint and [`REMOTE`] as output, then ends as `end` says.
fn scripted_server(lease: bool, end: End) -> (PathBuf, JoinHandle<()>) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("rusty-mux-tui-{}-{n}.sock", std::process::id()));
    let listener = UnixListener::bind(&path).expect("bind");
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut conn = Conn::new(stream);
        let block = BlockId::from_uuid(BLOCK.parse().expect("uuid"));
        assert!(matches!(conn.recv().expect("hello"), Message::Hello { .. }));
        conn.send(&Message::Welcome {
            features: Features::CHECKPOINT_V2,
            client: ClientId::from_uuid(Uuid::nil()),
        })
        .expect("welcome");
        while let Ok(message) = conn.recv() {
            match message {
                Message::Attach { req, .. } => {
                    conn.send(&Message::Done { req, created: None })
                        .expect("done");
                    attach(&mut conn, block);
                }
                Message::AcquireLease { req, .. } if !lease => {
                    conn.send(&Message::Failed {
                        req,
                        reason: "held by another client".into(),
                    })
                    .expect("failed");
                    if finish(&mut conn, block, end) {
                        return;
                    }
                }
                Message::AcquireLease { req, .. } => {
                    conn.send(&Message::Done { req, created: None })
                        .expect("done");
                }
                Message::Resize { req, .. } => {
                    conn.send(&Message::Done { req, created: None })
                        .expect("done");
                    if finish(&mut conn, block, end) {
                        return;
                    }
                }
                _ => {}
            }
        }
    });
    (path, server)
}

/// The checkpoint of a blank 80x24 terminal, Ready, then [`REMOTE`].
fn attach(conn: &mut Conn<std::os::unix::net::UnixStream>, block: BlockId) {
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
        data: REMOTE.to_vec(),
    })
    .expect("output");
}

/// Ends the attach; true when the server should stop reading.
fn finish(conn: &mut Conn<std::os::unix::net::UnixStream>, block: BlockId, end: End) -> bool {
    match end {
        End::Eof => true,
        End::Closed => {
            conn.send(&Message::Event(Event::BlockClosed { block }))
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
        let cmd = Command::new(env!("CARGO_BIN_EXE_rusty-mux"), "/")
            .arg(socket.as_os_str())
            .arg(BLOCK);
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
    let remote = find(REMOTE).unwrap_or_else(|| panic!("no remote output: {text:?}"));
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
    tui.master.write(&[0x1d]).expect("ctrl-]");
    let screen = tui.exit();
    server.join().expect("server");
    assert_restored(&screen, "detached from block");
}

//! The GUI client's source against a real server: a block's screen and
//! output reach its pane, typing needs the lease the focus brings, and the
//! window's layout changes are followed.

#![cfg(target_os = "linux")]

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use rusty_mux_client::Conn;
use rusty_mux_client::attach::{CONTROL, Requests};
use rusty_mux_core::Side;
use rusty_mux_gui::source::{Out, attach};
use rusty_mux_proto::{Launch, Message};
use rusty_mux_server::{Config, ConsoleHost, serve};
use rusty_term::gui::source::{Feed, PaneIo, Source, Tree};
use rusty_tokio::io::UnixListener;
use rusty_tokio::{Builder, LocalOptions};

const PATIENCE: Duration = Duration::from_secs(20);

/// Starts a server running `/bin/sh` on a fresh socket.
fn start_server() -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("rusty-mux-gui-{}-{n}.sock", std::process::id()));
    let config = Config {
        program: "/bin/sh".into(),
        args: Vec::new(),
        cwd: "/".into(),
        console_host: ConsoleHost::default(),
        state_dir: None,
        config_file: None,
    };
    let (ready_tx, ready_rx) = channel();
    let bind = path.clone();
    std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime");
        runtime.block_on(async move {
            let listener = UnixListener::bind(&bind).expect("bind");
            ready_tx.send(()).expect("ready");
            serve(listener, config).await.expect("serve");
        });
    });
    ready_rx.recv_timeout(PATIENCE).expect("server bound");
    path
}

/// The feed of `io` on a thread, so a test can wait on it with a deadline.
fn feed_of(io: PaneIo) -> (Box<dyn rusty_term::backend::BackendHandle>, Receiver<Feed>) {
    let (tx, rx) = channel();
    let mut feed = io.feed;
    std::thread::spawn(move || {
        for step in feed.by_ref() {
            if tx.send(step).is_err() {
                return;
            }
        }
    });
    (io.writer, rx)
}

/// Types `echo MARK$((n*2))` until the feed shows `MARKn*2`: typing is
/// dropped until the lease is granted.
fn type_until_echoed(
    writer: &mut dyn rusty_term::backend::BackendHandle,
    feed: &Receiver<Feed>,
    n: u32,
) {
    let deadline = Instant::now() + PATIENCE;
    let want = format!("MARK{}", n * 2);
    let mut seen = Vec::new();
    loop {
        writer
            .write(format!("echo MARK$(({n}*2))\r").as_bytes())
            .expect("write");
        let typed = Instant::now();
        while typed.elapsed() < Duration::from_millis(300) {
            if let Ok(Feed::Bytes(b)) = feed.recv_timeout(Duration::from_millis(50)) {
                seen.extend_from_slice(&b);
            }
            if String::from_utf8_lossy(&seen).contains(&want) {
                return;
            }
        }
        assert!(Instant::now() < deadline, "no echo of {want}: {seen:?}");
    }
}

#[test]
fn a_block_shows_its_screen_takes_typing_and_follows_the_layout() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let Tree::Pane(key) = attached.first else {
        panic!("one block first: {:?}", attached.first);
    };
    let io = attached.source.open(key, 80, 24).expect("opened");
    assert!(!io.answers_queries, "the server's terminal answers");
    let (mut writer, feed) = feed_of(io);
    let first = feed.recv_timeout(PATIENCE).expect("a first step");
    assert!(matches!(first, Feed::Screen(..)), "the screen comes first");
    type_until_echoed(writer.as_mut(), &feed, 1);

    // Another client splits the window: the layout follows.
    let (layouts_tx, layouts) = channel();
    let events = attached.events;
    let block = attached.block;
    std::thread::spawn(move || {
        events.run(|out| match out {
            Out::Layout(t) => layouts_tx.send(t).is_ok(),
            _ => true,
        })
    });
    let other = UnixStream::connect(&socket).expect("connect");
    let mut conn = Conn::new(other);
    conn.send(&Message::Hello {
        features: CONTROL,
        resume: None,
        takeover: None,
    })
    .expect("hello");
    let mut req = Requests {
        conn: &mut conn,
        next: 1,
        by: Some(Instant::now() + PATIENCE),
        cancel: &std::sync::atomic::AtomicBool::new(false),
    };
    req.welcome().expect("welcome");
    req.call(|req| Message::Split {
        req,
        block,
        side: Side::Right,
        launch: Launch::default(),
    })
    .expect("split");
    let split = layouts.recv_timeout(PATIENCE).expect("a layout");
    let Some(Tree::Split { a, b, .. }) = split else {
        panic!("a split: {split:?}");
    };
    assert_eq!(*a, Tree::Pane(key), "the first block keeps its key");
    let Tree::Pane(new) = *b else {
        panic!("a new block: {b:?}");
    };
    assert_ne!(new, key);

    // The focused block's lease is still this client's.
    type_until_echoed(writer.as_mut(), &feed, 2);

    // Focusing the new block takes the lease there.
    let (mut beside, beside_feed) = feed_of(attached.source.open(new, 40, 24).expect("opened"));
    attached.source.focus(new);
    type_until_echoed(beside.as_mut(), &beside_feed, 3);
}

/// The next session the run told, skipping layouts and focus moves.
fn next_session(outs: &Receiver<Out>) -> rusty_term::gui::source::Session {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if let Out::Session(s) = outs.recv_timeout(left).expect("a session") {
            return s;
        }
    }
}

#[test]
fn the_strip_shows_the_session_and_a_new_window_is_shown_when_made() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let (tx, outs) = channel();
    let events = attached.events;
    std::thread::spawn(move || events.run(|out| tx.send(out).is_ok()));
    let first = next_session(&outs);
    assert_eq!(first.windows.len(), 1);
    assert!(first.windows[0].shown);
    assert!(!first.host.is_empty());
    assert!(first.clients >= 1);

    // "+" makes a window with a block and selects it: the window follows.
    attached.source.new_window();
    let deadline = Instant::now() + PATIENCE;
    let shown = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if let Out::Layout(Some(t)) = outs.recv_timeout(left).expect("the new window") {
            break t;
        }
    };
    assert!(
        matches!(shown, Tree::Pane(_)),
        "one block in the new window: {shown:?}"
    );
    let now = loop {
        let s = next_session(&outs);
        if s.windows.len() == 2 && s.windows[1].shown {
            break s;
        }
    };

    // Picking the first tab goes back to it.
    attached.source.select(now.windows[0].key);
    loop {
        let s = next_session(&outs);
        if s.windows[0].shown {
            break;
        }
    }
}

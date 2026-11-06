//! The GUI client's source against a real server: a block's screen and
//! output reach its pane, typing needs the lease the focus brings, the
//! window's layout changes are followed, the window's own split and zoom
//! requests come back as layout and zoom, a session picked in the
//! switcher is shown, and a detached block is listed and put back.

#![cfg(target_os = "linux")]

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use rusty_mux_client::Conn;
use rusty_mux_client::attach::{CONTROL, Requests};
use rusty_mux_core::{Side, Size};
use rusty_mux_gui::source::{Out, attach};
use rusty_mux_proto::{Launch, Message};
use rusty_mux_server::{Config, ConsoleHost, serve};
use rusty_term::gui::source::{Act, Dir, Feed, PaneIo, Source, Tree};
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

#[test]
fn a_block_header_follows_its_folder() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let (tx, outs) = channel();
    let events = attached.events;
    std::thread::spawn(move || events.run(|out| tx.send(out).is_ok()));
    let key = attached.focus;
    let header = |want: &dyn Fn(&rusty_term::gui::source::BlockInfo) -> bool| {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if let Out::Blocks(blocks) = outs.recv_timeout(left).expect("a header") {
                let info = blocks.iter().find(|(k, _)| *k == key).map(|(_, i)| i);
                if info.is_some_and(want) {
                    return;
                }
            }
        }
    };
    header(&|i| i.cwd.as_deref() == Some("/") && i.shell.is_some());

    // Typing needs the lease, which the focused pane asks for.
    let io = attached
        .source
        .open(key, 80, 24)
        .expect("the focused block opens");
    let (mut writer, _feed) = feed_of(io);
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        // A shell tells its folder with OSC 7, which the server announces.
        writer
            .write(b"cd /tmp; printf '\\033]7;file://localhost/tmp\\007'\n")
            .expect("typed");
        std::thread::sleep(Duration::from_millis(200));
        let moved = outs.try_iter().any(|o| match o {
            Out::Blocks(b) => b
                .iter()
                .any(|(k, i)| *k == key && i.cwd.as_deref() == Some("/tmp")),
            _ => false,
        });
        if moved {
            return;
        }
    }
    panic!("the header never followed the move to /tmp");
}

#[test]
fn a_split_and_a_zoom_asked_by_the_window_come_back() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let Tree::Pane(key) = attached.first else {
        panic!("one block first: {:?}", attached.first);
    };
    let (outs_tx, outs) = channel();
    let events = attached.events;
    std::thread::spawn(move || events.run(|out| outs_tx.send(out).is_ok()));
    let next = |want: &dyn Fn(&Out) -> bool| loop {
        let out = outs.recv_timeout(PATIENCE).expect("an event");
        if want(&out) {
            return out;
        }
    };
    assert_eq!(
        next(&|o| matches!(o, Out::Zoom(_))),
        Out::Zoom(None),
        "nothing zoomed at first"
    );

    attached.source.act(Act::Split {
        key,
        dir: Dir::Vertical,
    });
    let Out::Layout(Some(Tree::Split { dir, a, .. })) =
        next(&|o| matches!(o, Out::Layout(Some(Tree::Split { .. }))))
    else {
        unreachable!("matched above");
    };
    assert_eq!(
        (dir, *a),
        (Dir::Vertical, Tree::Pane(key)),
        "the new block is on the right"
    );

    attached.source.act(Act::Zoom(key));
    assert_eq!(
        next(&|o| matches!(o, Out::Zoom(Some(_)))),
        Out::Zoom(Some(key))
    );
    attached.source.act(Act::Unzoom);
    assert_eq!(next(&|o| matches!(o, Out::Zoom(_))), Out::Zoom(None));
}

#[test]
fn a_session_picked_in_the_switcher_is_shown_on_the_same_connection() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let Tree::Pane(first) = attached.first else {
        panic!("one block first: {:?}", attached.first);
    };
    let (tx, outs) = channel();
    let events = attached.events;
    std::thread::spawn(move || events.run(|out| tx.send(out).is_ok()));

    // Another client makes a second session.
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
    let size = Size::new(80, 24).expect("a size");
    req.create(size, "other").expect("a second session");
    let listed = loop {
        let s = next_session(&outs);
        if let Some(c) = s.sessions.iter().find(|c| c.name == "other" && c.can_show) {
            break c.clone();
        }
    };
    assert!(!listed.current);

    attached.source.switch(listed.key);
    let deadline = Instant::now() + PATIENCE;
    let shown = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if let Out::Layout(Some(t)) = outs.recv_timeout(left).expect("its layout") {
            break t;
        }
    };
    assert!(
        matches!(shown, Tree::Pane(k) if k != first),
        "the other session's block: {shown:?}"
    );
    let now = next_session(&outs);
    assert_eq!(now.name, "other");
    assert!(
        now.sessions
            .iter()
            .any(|c| c.key == listed.key && c.current)
    );
    assert_eq!(now.windows.len(), 1, "only the other session's window");
}

/// A control client of its own on `socket`, welcomed.
fn control(socket: &std::path::Path) -> Conn<UnixStream> {
    let mut conn = Conn::new(UnixStream::connect(socket).expect("connect"));
    conn.send(&Message::Hello {
        features: CONTROL,
        resume: None,
        takeover: None,
    })
    .expect("hello");
    conn
}

#[test]
fn a_detached_block_is_listed_and_reattach_puts_it_back() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let Tree::Pane(key) = attached.first else {
        panic!("one block first: {:?}", attached.first);
    };
    let (tx, outs) = channel();
    let events = attached.events;
    std::thread::spawn(move || events.run(|out| tx.send(out).is_ok()));
    let next = |want: &dyn Fn(&Out) -> bool| loop {
        let out = outs.recv_timeout(PATIENCE).expect("an event");
        if want(&out) {
            return out;
        }
    };

    attached.source.act(Act::Split {
        key,
        dir: Dir::Vertical,
    });
    let Out::Layout(Some(Tree::Split { b, .. })) =
        next(&|o| matches!(o, Out::Layout(Some(Tree::Split { .. }))))
    else {
        unreachable!("matched above");
    };
    let Tree::Pane(new) = *b else {
        panic!("a block on the right: {b:?}");
    };

    // Another client detaches the new block (the GUI has no key for it).
    let mut conn = control(&socket);
    let mut req = Requests {
        conn: &mut conn,
        next: 1,
        by: Some(Instant::now() + PATIENCE),
        cancel: &std::sync::atomic::AtomicBool::new(false),
    };
    req.welcome().expect("welcome");
    let listed = req.call(|req| Message::List { req }).expect("a list");
    let blocks: Vec<_> = listed
        .iter()
        .filter_map(|m| match m {
            Message::Entry {
                entry: rusty_mux_proto::Entry::Block { block, .. },
                ..
            } => Some(*block),
            _ => None,
        })
        .collect();
    assert_eq!(blocks.len(), 2);
    let session = loop {
        let s = next_session(&outs);
        if s.windows
            .first()
            .is_some_and(|w| matches!(w.tree, Some(Tree::Split { .. })))
        {
            break s;
        }
    };
    assert!(session.detached.is_empty());
    // Detach each in turn until the GUI's `new` is the one out.
    let mut out = None;
    for block in blocks {
        req.call(|req| Message::DetachBlock { req, block })
            .expect("detached");
        let s = loop {
            let s = next_session(&outs);
            if !s.detached.is_empty() {
                break s;
            }
        };
        if s.detached[0].key == new {
            out = Some(s);
            break;
        }
        // The other one went: put it back and try the next.
        attached.source.act(Act::Reattach(s.detached[0].key));
        loop {
            if next_session(&outs).detached.is_empty() {
                break;
            }
        }
    }
    let s = out.expect("the new block detached");
    assert_eq!(s.detached.len(), 1);
    assert!(s.detached[0].since.is_some(), "timed from the change seen");
    assert!(!s.detached[0].name.is_empty());

    attached.source.act(Act::Reattach(new));
    assert!(
        matches!(
            next(&|o| matches!(o, Out::Layout(Some(Tree::Split { .. })))),
            Out::Layout(Some(_))
        ),
        "two blocks again"
    );
    loop {
        if next_session(&outs).detached.is_empty() {
            break;
        }
    }

    // With every block detached the window shows none, and lists both.
    let listed = req.call(|req| Message::List { req }).expect("a list");
    for m in listed {
        if let Message::Entry {
            entry: rusty_mux_proto::Entry::Block { block, .. },
            ..
        } = m
        {
            req.call(|req| Message::DetachBlock { req, block })
                .expect("detached");
        }
    }
    assert_eq!(next(&|o| matches!(o, Out::Empty)), Out::Empty);
    loop {
        if next_session(&outs).detached.len() == 2 {
            break;
        }
    }
    attached.source.act(Act::Reattach(new));
    assert_eq!(
        next(&|o| matches!(o, Out::Layout(_))),
        Out::Layout(Some(Tree::Pane(new))),
        "it fills the empty window"
    );
}

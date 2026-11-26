//! The GUI client's source against a real server: a block's screen and
//! output reach its pane, typing needs the lease the focus brings (asked
//! for again once another client gives it back), the
//! window's layout changes are followed, the window's own split and zoom
//! requests come back as layout and zoom, a session picked in the
//! switcher is shown, a detached block is listed and put back, and a
//! lost connection is taken over, or attached anew on Retry, saying when
//! the session shown is gone.

#![cfg(target_os = "linux")]

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use rusty_mux_client::Conn;
use rusty_mux_client::attach::{CONTROL, Requests};
use rusty_mux_core::{BlockId, InputMode, Side, Size};
use rusty_mux_gui::source::{Out, attach};
use rusty_mux_proto::{Entry, Features, Launch, Message};
use rusty_mux_server::{Config, ConsoleHost, serve};
use rusty_term::gui::source::{Act, Dir, Feed, Link, PaneIo, Show, Source, Take, Tree};
use rusty_tokio::io::UnixListener;
use rusty_tokio::{Builder, LocalOptions};

const PATIENCE: Duration = Duration::from_secs(20);

/// Starts a server running `/bin/sh` on a fresh socket.
fn start_server() -> PathBuf {
    start_server_in(None)
}

/// [`start_server`] keeping its state (and uploads) in `state_dir`.
fn start_server_in(state_dir: Option<PathBuf>) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("rusty-mux-gui-{}-{n}.sock", std::process::id()));
    let config = Config {
        program: "/bin/sh".into(),
        args: Vec::new(),
        cwd: "/".into(),
        console_host: ConsoleHost::default(),
        state_dir,
        protected: Default::default(),
        grapheme_clustering: false,
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

/// The feed of `io` on a thread, so a test can wait on it with a deadline;
/// where taking the block over stands is left out.
fn feed_of(io: PaneIo) -> (Box<dyn rusty_term::backend::BackendHandle>, Receiver<Feed>) {
    let (writer, feed, _) = feed_and_takes(io);
    (writer, feed)
}

/// [`feed_of`], with where taking the block over stands on its own.
fn feed_and_takes(
    io: PaneIo,
) -> (
    Box<dyn rusty_term::backend::BackendHandle>,
    Receiver<Feed>,
    Receiver<Take>,
) {
    let (tx, rx) = channel();
    let (takes_tx, takes) = channel();
    let mut feed = io.feed;
    std::thread::spawn(move || {
        for step in feed.by_ref() {
            if let Feed::Take(take) = step {
                let _ = takes_tx.send(take);
                continue;
            }
            if tx.send(step).is_err() {
                return;
            }
        }
    });
    (io.writer, rx, takes)
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
            Out::Layout(t, _) => layouts_tx.send(t).is_ok(),
            _ => true,
        })
    });
    let other = UnixStream::connect(&socket).expect("connect");
    let mut conn = Conn::new(other);
    conn.send(&Message::Hello {
        version: None,
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

#[test]
fn a_lease_another_client_holds_is_asked_for_again_once_it_is_free() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let block = attached.block;
    let mut other = control(&socket);
    let mut req = Requests {
        conn: &mut other,
        next: 1,
        by: Some(Instant::now() + PATIENCE),
        cancel: &std::sync::atomic::AtomicBool::new(false),
    };
    req.welcome().expect("welcome");
    req.call(|req| Message::Attach { req, block })
        .expect("attach");
    req.call(|req| Message::AcquireLease { req, block })
        .expect("the other client holds the lease");

    // The focused pane is refused the lease: its typing is dropped.
    let Tree::Pane(key) = attached.first else {
        panic!("one block first: {:?}", attached.first);
    };
    let (mut writer, feed) = feed_of(attached.source.open(key, 80, 24).expect("opened"));
    next_screen(&feed);
    writer.write(b"echo MARK$((7*2))\r").expect("write");
    let refused = Instant::now() + Duration::from_secs(1);
    while let Some(left) = refused.checked_duration_since(Instant::now()) {
        if let Ok(Feed::Bytes(b)) = feed.recv_timeout(left) {
            assert!(
                !String::from_utf8_lossy(&b).contains("MARK14"),
                "typed without the lease"
            );
        }
    }

    // Once it is given back, the pane asks again and types (handoff D115).
    req.call(|req| Message::ReleaseLease { req, block })
        .expect("release");
    type_until_echoed(writer.as_mut(), &feed, 8);
}

/// The next take-over state the pane told.
fn next_take(takes: &Receiver<Take>) -> Take {
    takes.recv_timeout(PATIENCE).expect("a take-over state")
}

/// Whether typing `echo MARKn*2` shows nothing for a second: the pane
/// holds no lease.
fn typing_is_dropped(
    writer: &mut dyn rusty_term::backend::BackendHandle,
    feed: &Receiver<Feed>,
    n: u32,
) -> bool {
    writer
        .write(format!("echo MARK$(({n}*2))\r").as_bytes())
        .expect("write");
    let want = format!("MARK{}", n * 2);
    let quiet = Instant::now() + Duration::from_secs(1);
    while let Some(left) = quiet.checked_duration_since(Instant::now()) {
        if let Ok(Feed::Bytes(b)) = feed.recv_timeout(left)
            && String::from_utf8_lossy(&b).contains(&want)
        {
            return false;
        }
    }
    true
}

#[test]
fn an_owner_only_block_is_taken_over_once_its_owner_grants_and_given_back() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let (layouts_tx, layouts) = channel();
    let events = attached.events;
    std::thread::spawn(move || {
        events.run(|out| match out {
            Out::Layout(t, _) => layouts_tx.send(t).is_ok(),
            _ => true,
        })
    });

    // An owner, as orch would be, starts an owner-only block beside the first.
    let mut owner = Conn::new(UnixStream::connect(&socket).expect("connect"));
    owner
        .send(&Message::Hello {
            version: None,
            features: CONTROL
                .union(Features::LAUNCH)
                .union(Features::OWNERSHIP)
                .union(Features::INPUT_GATE)
                .union(Features::LEASE_REQUESTS),
            resume: None,
            takeover: None,
        })
        .expect("hello");
    let mut req = Requests {
        conn: &mut owner,
        next: 1,
        by: Some(Instant::now() + PATIENCE),
        cancel: &std::sync::atomic::AtomicBool::new(false),
    };
    req.welcome().expect("welcome");
    let gated = Launch {
        own: true,
        input: InputMode::OwnerOnly,
        ..Launch::default()
    };
    let block = req
        .create_id(|req| Message::Split {
            req,
            block: attached.block,
            side: Side::Right,
            launch: gated,
        })
        .map(BlockId::from_uuid)
        .expect("an owned block");
    let split = layouts.recv_timeout(PATIENCE).expect("a layout");
    let Some(Tree::Split { b, .. }) = split else {
        panic!("a split: {split:?}");
    };
    let Tree::Pane(key) = *b else {
        panic!("the owned block: {b:?}");
    };

    // Focused, it is not taken: its input is its owner's.
    let (mut writer, feed, takes) =
        feed_and_takes(attached.source.open(key, 40, 24).expect("opened"));
    assert_eq!(next_take(&takes), Take::Rest);
    attached.source.focus(key);
    next_screen(&feed);
    assert!(typing_is_dropped(writer.as_mut(), &feed, 5));

    // Take over asks the owner; its grant gives this pane the input.
    attached.source.act(Act::TakeOver(key));
    assert_eq!(next_take(&takes), Take::Asking);
    let asker = loop {
        match req.recv().expect("the ask") {
            Message::LeaseRequested {
                block: b, client, ..
            } if b == block => break client,
            _ => {}
        }
    };
    let until = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64
        + 60_000;
    req.call(|req| Message::GrantLease {
        req,
        block,
        client: asker,
        until,
    })
    .expect("granted");
    assert_eq!(next_take(&takes), Take::Held);
    type_until_echoed(writer.as_mut(), &feed, 6);

    // Given back, typing is dropped again, focus or not.
    attached.source.act(Act::GiveBack(key));
    assert_eq!(next_take(&takes), Take::Rest);
    assert!(typing_is_dropped(writer.as_mut(), &feed, 7));

    // Taken over again on the live grant, no new ask of the owner, then
    // the owner revokes it.
    attached.source.act(Act::TakeOver(key));
    assert_eq!(next_take(&takes), Take::Asking, "the lease on its way");
    assert_eq!(next_take(&takes), Take::Held);
    req.call(|req| Message::RevokeLease {
        req,
        block,
        client: asker,
    })
    .expect("revoked");
    assert_eq!(next_take(&takes), Take::Ended);
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
        if let Out::Layout(Some(t), _) = outs.recv_timeout(left).expect("the new window") {
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
    let Out::Layout(Some(Tree::Split { dir, a, .. }), _) =
        next(&|o| matches!(o, Out::Layout(Some(Tree::Split { .. }), _)))
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
        version: None,
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
        if let Out::Layout(Some(t), _) = outs.recv_timeout(left).expect("its layout") {
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
    control_with(socket, CONTROL)
}

/// [`control`] with `features`.
fn control_with(socket: &std::path::Path, features: Features) -> Conn<UnixStream> {
    let mut conn = Conn::new(UnixStream::connect(socket).expect("connect"));
    conn.send(&Message::Hello {
        version: None,
        features,
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
    let Out::Layout(Some(Tree::Split { b, .. }), _) =
        next(&|o| matches!(o, Out::Layout(Some(Tree::Split { .. }), _)))
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
            next(&|o| matches!(o, Out::Layout(Some(Tree::Split { .. }), _))),
            Out::Layout(Some(_), _)
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
        next(&|o| matches!(o, Out::Layout(..))),
        Out::Layout(Some(Tree::Pane(new)), Vec::new()),
        "it fills the empty window"
    );
}

/// A socket forwarding to the server's, whose connections can be cut as
/// a broken link would, leaving the server's clients kept for a takeover.
struct Relay {
    path: PathBuf,
    open: std::sync::Arc<std::sync::Mutex<Vec<UnixStream>>>,
    target: std::sync::Arc<std::sync::Mutex<PathBuf>>,
}

impl Relay {
    fn new(socket: &std::path::Path) -> Self {
        let path = socket.with_extension("relay");
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind the relay");
        let open = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let target = std::sync::Arc::new(std::sync::Mutex::new(socket.to_owned()));
        let (to, kept) = (std::sync::Arc::clone(&target), std::sync::Arc::clone(&open));
        std::thread::spawn(move || {
            for near in listener.incoming() {
                let far = UnixStream::connect(&*to.lock().expect("relay"));
                let (Ok(near), Ok(far)) = (near, far) else {
                    return;
                };
                let pipe = |mut from: UnixStream, mut to: UnixStream| {
                    std::thread::spawn(move || {
                        let _ = std::io::copy(&mut from, &mut to);
                        let _ = to.shutdown(std::net::Shutdown::Both);
                    });
                };
                let clone = |s: &UnixStream| s.try_clone().expect("clone");
                kept.lock()
                    .expect("relay")
                    .extend([clone(&near), clone(&far)]);
                pipe(clone(&near), clone(&far));
                pipe(far, near);
            }
        });
        Self { path, open, target }
    }

    /// Ends every connection and sends new ones to `socket`, as if
    /// another server took the address.
    fn retarget(&self, socket: &std::path::Path) {
        *self.target.lock().expect("relay") = socket.to_owned();
        self.cut();
    }

    /// Ends every connection made so far.
    fn cut(&self) {
        for s in self.open.lock().expect("relay").drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// The next `out` that `want` takes, skipping the rest.
fn next_out(outs: &Receiver<Out>, want: impl Fn(&Out) -> bool) -> Out {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let out = outs.recv_timeout(left).expect("an event");
        if want(&out) {
            return out;
        }
    }
}

/// Waits for the block's screen to be read again.
fn next_screen(feed: &Receiver<Feed>) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if let Feed::Screen(..) = feed.recv_timeout(left).expect("a screen") {
            return;
        }
    }
}

#[test]
fn a_lost_link_is_taken_over_and_the_open_block_goes_on() {
    let socket = start_server();
    let relay = Relay::new(&socket);
    let attached = attach(&relay.path, None).expect("attached");
    let Tree::Pane(key) = attached.first else {
        panic!("one block first: {:?}", attached.first);
    };
    let (mut writer, feed) = feed_of(attached.source.open(key, 80, 24).expect("opened"));
    next_screen(&feed);
    type_until_echoed(writer.as_mut(), &feed, 1);
    let (tx, outs) = channel();
    let events = attached.events;
    std::thread::spawn(move || events.run(|out| tx.send(out).is_ok()));

    relay.cut();
    let link = |o: &Out| matches!(o, Out::Link(_));
    assert!(matches!(
        next_out(&outs, link),
        Out::Link(Link::Reconnecting { left: 30, .. })
    ));
    let back = next_out(&outs, |o| {
        matches!(o, Out::Link(Link::Back { .. } | Link::Down { .. }))
    });
    assert_eq!(back, Out::Link(Link::Back { kept: true }));
    assert_eq!(
        next_out(&outs, |o| matches!(o, Out::Layout(..))),
        Out::Layout(Some(Tree::Pane(key)), Vec::new())
    );
    // The same pane: its screen read again, its lease kept or taken back.
    next_screen(&feed);
    type_until_echoed(writer.as_mut(), &feed, 2);
}

#[test]
fn a_client_the_server_let_go_waits_for_retry_and_attaches_anew() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let Tree::Pane(key) = attached.first else {
        panic!("one block first: {:?}", attached.first);
    };
    let (writer, feed) = feed_of(attached.source.open(key, 80, 24).expect("opened"));
    next_screen(&feed);
    let (tx, outs) = channel();
    let events = attached.events;
    std::thread::spawn(move || events.run(|out| tx.send(out).is_ok()));

    // Another client ends the GUI's control client: it is not kept.
    let mut conn = Conn::new(UnixStream::connect(&socket).expect("connect"));
    conn.send(&Message::Hello {
        version: None,
        features: CONTROL.union(Features::LABELS),
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
    let me = req.welcome().expect("welcome").client;
    let controls: Vec<_> = req
        .call(|req| Message::List { req })
        .expect("a list")
        .into_iter()
        .filter_map(|m| match m {
            Message::Entry {
                entry:
                    Entry::Client {
                        client,
                        attached: None,
                        ..
                    },
                ..
            } if client != me => Some(client),
            _ => None,
        })
        .collect();
    assert_eq!(controls.len(), 1, "the GUI's control client");
    req.call(|req| Message::Disconnect {
        req,
        client: controls[0],
    })
    .expect("disconnected");

    let link = |o: &Out| matches!(o, Out::Link(Link::Down { .. } | Link::Back { .. }));
    assert_eq!(
        next_out(&outs, link),
        Out::Link(Link::Down { trying: false })
    );
    attached.source.retry();
    assert_eq!(
        next_out(&outs, link),
        Out::Link(Link::Down { trying: true })
    );
    assert_eq!(next_out(&outs, link), Out::Link(Link::Back { kept: false }));
    assert_eq!(
        next_out(&outs, |o| matches!(o, Out::Layout(..))),
        Out::Layout(Some(Tree::Pane(key)), Vec::new())
    );
    // The window opens its block again, as a new client.
    drop((writer, feed));
    let (mut writer, feed) = feed_of(attached.source.open(key, 80, 24).expect("reopened"));
    next_screen(&feed);
    type_until_echoed(writer.as_mut(), &feed, 3);
}

#[test]
fn a_session_gone_after_a_reconnect_is_said_and_another_can_be_shown() {
    let socket = start_server();
    let relay = Relay::new(&socket);
    let attached = attach(&relay.path, None).expect("attached");
    let (tx, outs) = channel();
    let events = attached.events;
    std::thread::spawn(move || events.run(|out| tx.send(out).is_ok()));

    // Another server, with a session of its own, takes the address.
    let other = start_server();
    let mut conn = control(&other);
    let mut req = Requests {
        conn: &mut conn,
        next: 1,
        by: Some(Instant::now() + PATIENCE),
        cancel: &std::sync::atomic::AtomicBool::new(false),
    };
    req.welcome().expect("welcome");
    req.create(Size::new(80, 24).expect("a size"), "elsewhere")
        .expect("a session");
    relay.retarget(&other);

    // It doesn't know this client: Retry attaches anew, and the session
    // the window showed is gone.
    let link = |o: &Out| matches!(o, Out::Link(Link::Down { trying: false }));
    next_out(&outs, link);
    attached.source.retry();
    let ended = next_out(&outs, |o| matches!(o, Out::Ended(_) | Out::Layout(..)));
    let Out::Ended(name) = ended else {
        panic!("the session ended: {ended:?}");
    };
    assert!(!name.is_empty());
    let Out::Session(strip) = next_out(&outs, |o| matches!(o, Out::Session(_))) else {
        unreachable!("matched above");
    };
    assert_eq!(strip.name, name, "the strip keeps the ended session's name");
    assert!(strip.windows.is_empty());
    let pick = strip
        .sessions
        .iter()
        .find(|c| c.name == "elsewhere" && c.can_show && !c.current)
        .expect("the other server's session");

    attached.source.switch(pick.key);
    assert!(matches!(
        next_out(&outs, |o| matches!(o, Out::Layout(..))),
        Out::Layout(Some(Tree::Pane(_)), _)
    ));
}

#[test]
fn a_folder_is_listed_over_the_blocks_connection_once_it_holds_the_lease() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let Tree::Pane(key) = attached.first else {
        panic!("one block first: {:?}", attached.first);
    };
    let (mut writer, feed) = feed_of(attached.source.open(key, 80, 24).expect("opened"));
    assert!(attached.source.lists(key), "the server lists folders");
    assert!(!attached.source.lists(key + 99), "no such pane");
    // Listing needs the lease, as typing does.
    type_until_echoed(writer.as_mut(), &feed, 1);

    let ask = |path: Option<&str>| {
        let (tx, rx) = channel();
        attached.source.list_dir(
            key,
            path.map(str::to_owned),
            Box::new(move |r| drop(tx.send(r))),
        );
        rx.recv_timeout(PATIENCE).expect("an answer")
    };
    let here = ask(None).expect("the block's folder");
    assert_eq!(here.lines().next(), Some("/"), "{here}");
    let tmp = std::env::temp_dir().join(format!("rusty-mux-gui-list-{}", std::process::id()));
    std::fs::create_dir_all(tmp.join("sub")).expect("mkdir");
    std::fs::write(tmp.join("a.txt"), b"x").expect("write");
    let listed = ask(tmp.to_str()).expect("a folder");
    let kinds: Vec<_> = listed.lines().skip(1).collect();
    assert_eq!(kinds, ["d\tsub", "f\ta.txt"], "{listed}");
    let refused = ask(Some("/no/such/folder/here")).expect_err("no such folder");
    assert!(refused.contains("terminal.list-dir"), "{refused}");
    std::fs::remove_dir_all(&tmp).expect("cleanup");
}

#[test]
fn a_forward_to_a_port_nothing_listens_on_tells_the_window_why() {
    use std::io::Read;
    use std::net::TcpStream;
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let Tree::Pane(key) = attached.first else {
        panic!("one block first: {:?}", attached.first);
    };
    let (mut writer, feed) = feed_of(attached.source.open(key, 80, 24).expect("opened"));
    assert!(attached.source.forwards(key), "the server tunnels ports");
    assert_eq!(
        attached.source.host_id(key).as_deref(),
        socket.canonicalize().expect("canonical").to_str(),
        "the server's socket names its host"
    );
    assert_eq!(attached.source.host_id(key + 99), None, "no such pane");
    type_until_echoed(writer.as_mut(), &feed, 1);
    // The client listens on the remote port's number when it is free, and the
    // server dials this machine too: hold the number while the forward
    // listens elsewhere, then let it go so nothing answers the dial.
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let dead = held.local_addr().expect("addr").port();
    let (tx, rx) = channel();
    attached
        .source
        .forward(key, dead, Box::new(move |r| drop(tx.send(r))));
    let bound = rx
        .recv_timeout(PATIENCE)
        .expect("an answer")
        .expect("listening");
    assert_ne!(bound.local, dead);
    drop(held);
    let mut local = TcpStream::connect(("127.0.0.1", bound.local)).expect("the local port");
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if let Ok(Feed::ForwardRefused { port, reason }) =
            feed.recv_timeout(Duration::from_millis(50))
        {
            assert_eq!(port, dead);
            assert!(!reason.is_empty());
            // The browser's connection is closed too.
            assert_eq!(local.read(&mut [0u8; 1]).unwrap_or(0), 0);
            return;
        }
    }
    panic!("no refusal reached the window");
}

/// Reads `feed` until a clipboard read arrives, its selection (C54).
fn next_clipboard_read(feed: &Receiver<Feed>) -> bool {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if let Ok(Feed::ClipboardRead { primary }) = feed.recv_timeout(Duration::from_millis(50)) {
            return primary;
        }
    }
    panic!("no clipboard read");
}

/// Reads `feed` until its output holds `needle`.
fn bytes_until(feed: &Receiver<Feed>, needle: &str) {
    let deadline = Instant::now() + PATIENCE;
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        if let Ok(Feed::Bytes(b)) = feed.recv_timeout(Duration::from_millis(50)) {
            seen.extend_from_slice(&b);
        }
        if String::from_utf8_lossy(&seen).contains(needle) {
            return;
        }
    }
    panic!("no {needle:?} in {seen:?}");
}

#[test]
fn a_clipboard_read_reaches_the_window_and_its_answer_reaches_the_program() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let Tree::Pane(key) = attached.first else {
        panic!("one block first: {:?}", attached.first);
    };
    let io = attached.source.open(key, 80, 24).expect("opened");
    assert!(
        io.serves_clipboard,
        "the window answers reads, not the grid"
    );
    let (mut writer, feed) = feed_of(io);
    type_until_echoed(writer.as_mut(), &feed, 1);

    writer
        .write(b"printf '\\033]52;c;?\\007'; read x\r")
        .expect("typed");
    assert!(!next_clipboard_read(&feed), "the clipboard, not primary");
    writer.clipboard_reply(false, Some("hi".into()));
    // The program's terminal echoes what it was sent: "hi" in base64.
    bytes_until(&feed, "52;c;aGk=");

    // A denial is an empty answer, so the program is not left waiting.
    writer
        .write(b"\rprintf '\\033]52;p;?\\007'; read y\r")
        .expect("typed");
    assert!(next_clipboard_read(&feed), "primary this time");
    writer.clipboard_reply(true, None);
    bytes_until(&feed, "52;p;^G");
}

/// Splits the window of `attached` and returns the pane keys `(first, new)`,
/// with the events of the run on `outs`.
fn split_window(
    attached: rusty_mux_gui::source::Attached,
) -> (u64, u64, Receiver<Out>, rusty_mux_gui::source::MuxSource) {
    let Tree::Pane(first) = attached.first else {
        panic!("one block first: {:?}", attached.first);
    };
    let (tx, outs) = channel();
    let events = attached.events;
    std::thread::spawn(move || events.run(|out| tx.send(out).is_ok()));
    attached.source.act(Act::Split {
        key: first,
        dir: Dir::Vertical,
    });
    let Out::Layout(Some(Tree::Split { b, .. }), _) = next_out(&outs, |o| {
        matches!(o, Out::Layout(Some(Tree::Split { .. }), _))
    }) else {
        unreachable!("matched above");
    };
    let Tree::Pane(new) = *b else {
        panic!("a block on the right: {b:?}");
    };
    (first, new, outs, attached.source)
}

#[test]
fn a_window_of_floats_alone_stays_open_and_shows_them() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let Tree::Pane(key) = attached.first else {
        panic!("one block first: {:?}", attached.first);
    };
    let (tx, outs) = channel();
    let events = attached.events;
    std::thread::spawn(move || events.run(|out| tx.send(out).is_ok()));

    // The only block floats: no tile is left, and the window is not closed.
    attached.source.act(Act::Float(key));
    let deadline = Instant::now() + PATIENCE;
    let (tree, floats) = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match outs.recv_timeout(left).expect("a layout with the float") {
            Out::Layout(None, _) => panic!("a window of floats alone was closed"),
            Out::Layout(Some(tree), floats) if !floats.is_empty() => break (tree, floats),
            _ => {}
        }
    };
    assert_eq!(floats.len(), 1);
    assert_eq!(floats[0].key, key);
    assert_eq!(tree, Tree::Pane(key), "the float stands for the tile");
}

#[test]
fn a_block_moving_between_detached_and_floating_tells_the_strip() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let (_, _, outs, source) = split_window(attached);

    // Another client detaches one block: the strip lists it, the other stays
    // tiled.
    let mut conn = control(&socket);
    let mut req = Requests {
        conn: &mut conn,
        next: 1,
        by: Some(Instant::now() + PATIENCE),
        cancel: &std::sync::atomic::AtomicBool::new(false),
    };
    req.welcome().expect("welcome");
    let listed = req.call(|req| Message::List { req }).expect("a list");
    let block = listed
        .iter()
        .find_map(|m| match m {
            Message::Entry {
                entry: rusty_mux_proto::Entry::Block { block, .. },
                ..
            } => Some(*block),
            _ => None,
        })
        .expect("a block");
    req.call(|req| Message::DetachBlock { req, block })
        .expect("detached");
    let out = loop {
        if let Some(d) = next_session(&outs).detached.first() {
            break d.key;
        }
    };
    // It floats: the window is told the float, and the strip that it is no
    // longer on the detached list, though no layout changed.
    source.act(Act::Float(out));
    next_out(&outs, |o| matches!(o, Out::Floats(f) if !f.is_empty()));
    loop {
        if next_session(&outs).detached.is_empty() {
            break;
        }
    }
}

#[test]
fn a_place_the_server_refuses_snaps_the_float_back() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let (first, new, outs, source) = split_window(attached);

    // Another client closes a block the window still has a key for: the
    // server refuses to place it, however fast it answers.
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
    // One of the two goes; the window is left with the other.
    req.call(|req| Message::CloseBlock {
        req,
        block: blocks[0],
    })
    .expect("closed");
    let Out::Layout(Some(Tree::Pane(left)), _) =
        next_out(&outs, |o| matches!(o, Out::Layout(Some(Tree::Pane(_)), _)))
    else {
        unreachable!("matched above");
    };
    let gone = if left == first { new } else { first };
    source.act(Act::Place {
        key: gone,
        at: rusty_term::gui::source::FloatAt {
            x: 10,
            y: 10,
            w: 50,
            h: 50,
        },
    });
    assert_eq!(
        next_out(&outs, |o| matches!(o, Out::Floats(_))),
        Out::Floats(Vec::new()),
        "told where it is"
    );
}

#[test]
fn a_block_is_reattached_to_a_window_of_floats_alone() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let (first, new, outs, source) = split_window(attached);

    // One block is detached; the other floats and has the focus: nothing is
    // tiled, and a float is no block to place beside.
    let mut conn = control(&socket);
    let mut req = Requests {
        conn: &mut conn,
        next: 1,
        by: Some(Instant::now() + PATIENCE),
        cancel: &std::sync::atomic::AtomicBool::new(false),
    };
    req.welcome().expect("welcome");
    let listed = req.call(|req| Message::List { req }).expect("a list");
    let block = listed
        .iter()
        .find_map(|m| match m {
            Message::Entry {
                entry: rusty_mux_proto::Entry::Block { block, .. },
                ..
            } => Some(*block),
            _ => None,
        })
        .expect("a block");
    req.call(|req| Message::DetachBlock { req, block })
        .expect("detached");
    let detached = loop {
        if let Some(d) = next_session(&outs).detached.first() {
            break d.key;
        }
    };
    let floated = if detached == first { new } else { first };
    source.act(Act::Float(floated));
    next_out(&outs, |o| matches!(o, Out::Layout(_, f) if !f.is_empty()));

    source.act(Act::Reattach(detached));
    let Out::Layout(Some(tree), floats) = next_out(
        &outs,
        |o| matches!(o, Out::Layout(Some(Tree::Pane(k)), f) if *k == detached && !f.is_empty()),
    ) else {
        unreachable!("matched above");
    };
    assert_eq!(tree, Tree::Pane(detached), "tiled again, the float stays");
    assert_eq!(floats[0].key, floated);
}

#[test]
fn a_gui_attached_to_a_float_reattaches_beside_no_float() {
    let socket = start_server();
    // A first window with its block, made by a GUI kept open aside.
    let _seed = attach(&socket, None).expect("seeded");
    // A window of one float and one detached block, before the GUI is there.
    let mut conn = control_with(&socket, rusty_mux_client::attach::FLOATING);
    let mut req = Requests {
        conn: &mut conn,
        next: 1,
        by: Some(Instant::now() + PATIENCE),
        cancel: &std::sync::atomic::AtomicBool::new(false),
    };
    req.welcome().expect("welcome");
    let block_of = |m: &Message| match m {
        Message::Entry {
            entry: rusty_mux_proto::Entry::Block { block, .. },
            ..
        } => Some(*block),
        _ => None,
    };
    let first = req
        .call(|req| Message::List { req })
        .expect("a list")
        .iter()
        .find_map(block_of)
        .expect("a block");
    req.call(|req| Message::Split {
        req,
        block: first,
        side: Side::Right,
        launch: Launch::default(),
    })
    .expect("split");
    let both: Vec<BlockId> = req
        .call(|req| Message::List { req })
        .expect("a list")
        .iter()
        .filter_map(block_of)
        .collect();
    assert_eq!(both.len(), 2);
    let (float, away) = (both[0], both[1]);
    let at = rusty_mux_core::FloatRect::new(20, 20, 60, 60).expect("a float");
    req.call(|req| Message::PlaceFloat {
        req,
        block: float,
        at,
    })
    .expect("floated");
    req.call(|req| Message::DetachBlock { req, block: away })
        .expect("detached");

    // The GUI attaches straight to the float.
    let attached = attach(&socket, Some(float)).expect("attached");
    let (tx, outs) = channel();
    let events = attached.events;
    std::thread::spawn(move || events.run(|out| tx.send(out).is_ok()));
    let detached = loop {
        if let Some(d) = next_session(&outs).detached.first() {
            break d.key;
        }
    };
    attached.source.act(Act::Reattach(detached));
    next_out(
        &outs,
        |o| matches!(o, Out::Layout(Some(Tree::Pane(k)), f) if *k == detached && !f.is_empty()),
    );
}

#[test]
fn a_place_that_changes_nothing_or_goes_nowhere_still_tells_the_floats() {
    let socket = start_server();
    let attached = attach(&socket, None).expect("attached");
    let (_, new, outs, source) = split_window(attached);
    source.act(Act::Float(new));
    let Out::Floats(floats) = next_out(&outs, |o| matches!(o, Out::Floats(f) if !f.is_empty()))
    else {
        unreachable!("matched above");
    };
    let at = floats[0].at;

    // Dropped where it is: the server says Done and changes nothing.
    source.act(Act::Place { key: new, at });
    assert_eq!(
        next_out(&outs, |o| matches!(o, Out::Floats(_))),
        Out::Floats(floats),
        "told where it is, so it does not wait"
    );

    // Dropped under the least size: nothing is sent, and it still snaps back.
    let tiny = rusty_term::gui::source::FloatAt {
        x: 0,
        y: 0,
        w: 5,
        h: 5,
    };
    source.act(Act::Place { key: new, at: tiny });
    let Out::Floats(told) = next_out(&outs, |o| matches!(o, Out::Floats(_))) else {
        unreachable!("matched above");
    };
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].at, at);
}

#[test]
fn a_window_asks_another_to_show_a_session_and_the_asked_hears_who() {
    let socket = start_server();
    let a = attach(&socket, None).expect("a");
    let (a_tx, a_outs) = channel();
    let a_events = a.events;
    std::thread::spawn(move || a_events.run(|out| a_tx.send(out).is_ok()));
    // A second session for the first window to hand over.
    let mut other = control(&socket);
    let mut req = Requests {
        conn: &mut other,
        next: 1,
        by: Some(Instant::now() + PATIENCE),
        cancel: &std::sync::atomic::AtomicBool::new(false),
    };
    req.welcome().expect("welcome");
    req.create(Size::new(80, 24).expect("a size"), "other")
        .expect("a second session");
    let b = attach(&socket, None).expect("b");
    let (b_tx, b_outs) = channel();
    let b_events = b.events;
    std::thread::spawn(move || b_events.run(|out| b_tx.send(out).is_ok()));

    // Each sees the other as a window, and the session to hand over.
    let (peer, session) = loop {
        let s = next_session(&a_outs);
        let session = s.sessions.iter().find(|c| c.name == "other");
        if let (Some(p), Some(c)) = (s.peers.first(), session) {
            break (p.clone(), c.key);
        }
    };
    assert!(peer.label.starts_with("Window "), "{}", peer.label);

    assert!(a.source.show_in(session, Show::Window(peer.key)));
    let Out::Shown(_, by) = next_out(&b_outs, |o| matches!(o, Out::Shown(..))) else {
        unreachable!()
    };
    assert!(by.starts_with("Window "), "{by}");

    assert!(a.source.show_in(session, Show::All));
    next_out(&b_outs, |o| matches!(o, Out::Shown(..)));
}

#[test]
fn a_file_is_sent_to_the_blocks_host_once_the_pane_holds_the_lease() {
    let state = std::env::temp_dir().join(format!("rusty-mux-gui-up-{}", std::process::id()));
    std::fs::create_dir_all(&state).expect("mkdir");
    let socket = start_server_in(Some(state.clone()));
    let attached = attach(&socket, None).expect("attached");
    let Tree::Pane(key) = attached.first else {
        panic!("one block first: {:?}", attached.first);
    };
    let (mut writer, feed) = feed_of(attached.source.open(key, 80, 24).expect("opened"));
    assert!(attached.source.uploads(key), "the server takes files");
    assert!(!attached.source.uploads(key + 99), "no such pane");
    type_until_echoed(writer.as_mut(), &feed, 1);

    let ask = |path: &std::path::Path| {
        let (tx, rx) = channel();
        attached
            .source
            .upload(key, path.to_owned(), Box::new(move |r| drop(tx.send(r))));
        rx.recv_timeout(PATIENCE).expect("an answer")
    };
    let local = state.join("local.bin");
    let body: Vec<u8> = (0..(3 << 20) + 5).map(|i| i as u8).collect();
    std::fs::write(&local, &body).expect("write");
    let sent = ask(&local).expect("sent");
    assert!(sent.ends_with("local.bin"), "{sent}");
    assert_eq!(std::fs::read(&sent).expect("on the server"), body);

    let missing = ask(&state.join("nope")).expect_err("no such file");
    assert!(missing.contains("nope"), "{missing}");
    let folder = ask(&state).expect_err("a folder is not a file");
    assert!(folder.contains("not a file"), "{folder}");
    // A FIFO would block the open for ever: refused before it is opened.
    let fifo = state.join("fifo");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if made.is_ok_and(|s| s.success()) {
        let refused = ask(&fifo).expect_err("a FIFO is not a file");
        assert!(refused.contains("not a file"), "{refused}");
    }
    std::fs::remove_dir_all(&state).expect("cleanup");
}

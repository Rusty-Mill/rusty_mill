//! One server window as the GUI window's session source.
//!
//! [`attach`] says Hello on a control connection, picks the window and
//! reads its layout. [`MuxSource`] opens a connection per shown block and
//! attaches it; the window drops a pane to detach its block. The focus is
//! the window's on the server (C41): a move here is told with
//! `FocusBlock`, a move by another client comes back as `FocusChanged`.
//! Only the focused block's lease is held, so other clients can type into
//! the rest, and only a leased block is resized (pack D102).

use std::collections::HashMap;
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver as Inbox, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

#[cfg(windows)]
use platform_windows::WindowsUnixStream as UnixStream;
use rusty_mux_client::Conn;
use rusty_mux_client::attach::{
    CONTROL, HANDSHAKE, Missed, Requests, clone, connect, first_session_name, reach,
};
use rusty_mux_client::checkpoint::Receiver;
use rusty_mux_core::{BlockId, Event, Size, WindowId};
use rusty_mux_proto::{Features, Message};
use rusty_term::backend::BackendHandle;
use rusty_term::gui::source::{Feed, PaneIo, Remote, Source, Tree};

use crate::keys::Keys;
use crate::tree::tree;

/// Never set: the handshakes here are not cancelled.
static NEVER: AtomicBool = AtomicBool::new(false);

/// Feed steps a block's reader keeps ahead of the window's parser.
const FEED_AHEAD: usize = 16;

/// The size a new block gets when there is none to attach to.
const FIRST_SIZE: (u16, u16) = (80, 24);

/// A server window, attached: its source, the layout to show first and
/// the control connection that tells of changes.
pub struct Attached {
    /// The block that took the focus, and its pane key.
    pub block: BlockId,
    pub focus: u64,
    pub source: MuxSource,
    pub first: Tree,
    pub events: Events,
}

/// Connects to the server at `socket` and picks `block`'s window, or that
/// of a new session's block without one; that block takes the focus.
pub fn attach(socket: &Path, block: Option<BlockId>) -> Result<Attached, String> {
    let stream = connect(socket)?;
    let mut conn = Conn::new(clone(&stream)?);
    conn.send(&Message::Hello {
        features: CONTROL,
        resume: None,
        takeover: None,
    })
    .map_err(|e| e.to_string())?;
    let mut req = Requests {
        conn: &mut conn,
        next: 1,
        by: Some(Instant::now() + HANDSHAKE),
        cancel: &NEVER,
    };
    let welcomed = req.welcome()?;
    if !welcomed
        .features
        .contains(Features::INSPECT.union(Features::LAYOUT))
    {
        return Err("the server cannot show a window's layout".into());
    }
    let block = match block {
        Some(block) => block,
        None => {
            let size = Size::new(FIRST_SIZE.0, FIRST_SIZE.1).map_err(|e| e.to_string())?;
            req.create(size, first_session_name(welcomed.features))?
        }
    };
    let (window, shape) = req.place(block)?;
    let window = window.ok_or("the block is in no window")?;
    let next = req.next;
    let layout = shape.layout.ok_or("the window shows no blocks")?;
    let state = Arc::new(State {
        socket: socket.to_owned(),
        control: Mutex::new(Conn::new(clone(&stream)?)),
        next: AtomicU64::new(next),
        tells_focus: welcomed.features.contains(Features::FOCUS),
        shown: Mutex::new(Shown::default()),
    });
    let (first, focus) = {
        let mut shown = state.shown();
        let first = tree(&layout, &mut shown.keys);
        let focus = shown.keys.key(block);
        shown.focus = Some(focus);
        (first, focus)
    };
    state.tell_focus(block);
    Ok(Attached {
        block,
        focus,
        source: MuxSource(Arc::clone(&state)),
        first,
        events: Events {
            state,
            conn,
            window,
        },
    })
}

/// What the client and the window share.
struct State {
    socket: PathBuf,
    control: Mutex<Conn<UnixStream>>,
    next: AtomicU64,
    /// The server takes `FocusBlock` (C41).
    tells_focus: bool,
    shown: Mutex<Shown>,
}

/// The blocks shown and which has the focus.
#[derive(Default)]
struct Shown {
    keys: Keys,
    /// Each open pane's block connection, kept weakly: the window owns it.
    blocks: HashMap<u64, std::sync::Weak<Block>>,
    focus: Option<u64>,
}

impl State {
    fn shown(&self) -> MutexGuard<'_, Shown> {
        self.shown.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tells the server the window's focus moved to `block`.
    fn tell_focus(&self, block: BlockId) {
        if !self.tells_focus {
            return;
        }
        let req = self.next.fetch_add(1, Ordering::Relaxed);
        let control = &mut self.control.lock().unwrap_or_else(PoisonError::into_inner);
        // A dead control connection ends the events reader, which closes
        // the window.
        let _ = control.send(&Message::FocusBlock { req, block });
    }

    /// Moves the focus to `key`, its block's lease with it.
    fn focus(&self, key: u64) {
        let mut shown = self.shown();
        shown.focus = Some(key);
        for (&k, block) in &shown.blocks {
            if let Some(block) = block.upgrade() {
                block.want_lease(k == key);
            }
        }
    }
}

/// The server window's blocks, opened on demand by the GUI window.
pub struct MuxSource(Arc<State>);

impl Source for MuxSource {
    fn open(&self, key: u64, cols: u16, rows: u16) -> std::io::Result<PaneIo> {
        let block = self
            .0
            .shown()
            .keys
            .block(key)
            .ok_or_else(|| std::io::Error::other(format!("no block has key {key}")))?;
        let (pane, feed) =
            Block::open(&self.0.socket, block, (cols, rows)).map_err(std::io::Error::other)?;
        let mut shown = self.0.shown();
        shown.blocks.retain(|_, b| b.strong_count() > 0);
        shown.blocks.insert(key, Arc::downgrade(&pane.0));
        pane.0.want_lease(shown.focus == Some(key));
        Ok(PaneIo {
            writer: Box::new(pane),
            feed: Box::new(feed.into_iter()),
            // The server's own terminal answers the program's queries.
            answers_queries: false,
        })
    }

    fn focus(&self, key: u64) {
        let block = self.0.shown().keys.block(key);
        if let Some(block) = block {
            self.0.tell_focus(block);
            self.0.focus(key);
        }
    }
}

/// The control connection's events for the window.
pub struct Events {
    state: Arc<State>,
    conn: Conn<UnixStream>,
    window: WindowId,
}

impl Events {
    /// Follows the window on a thread of its own: its layout and focus go
    /// to `remote`, and its end, or the connection's, closes the window.
    pub fn follow(self, remote: Remote) {
        std::thread::spawn(move || self.run(|t| remote.layout(t), |k| remote.focus(k)));
    }

    /// Follows the window until it or the connection ends, or a call is
    /// `false`: each layout goes to `layout`, `None` last, and each focus
    /// another client moves to `focus`.
    pub fn run(
        mut self,
        mut layout: impl FnMut(Option<Tree>) -> bool,
        mut focus: impl FnMut(u64) -> bool,
    ) {
        while let Ok(message) = self.conn.recv() {
            let Message::Event { event, .. } = message else {
                continue;
            };
            match event {
                Event::LayoutChanged {
                    window,
                    layout: layout_of,
                    ..
                } if window == self.window => {
                    let shown = layout_of.map(|l| tree(&l, &mut self.state.shown().keys));
                    let ended = shown.is_none();
                    if !layout(shown) || ended {
                        return;
                    }
                }
                Event::WindowClosed { window } if window == self.window => break,
                Event::FocusChanged {
                    window,
                    block: Some(block),
                } if window == self.window => {
                    let key = self.state.shown().keys.key(block);
                    self.state.focus(key);
                    if !focus(key) {
                        return;
                    }
                }
                _ => {}
            }
        }
        layout(None);
    }
}

/// One block's connection, shared by the pane's writer and its reader.
struct Block {
    block: BlockId,
    stream: UnixStream,
    conn: Mutex<Conn<UnixStream>>,
    next: AtomicU64,
    lease: Mutex<Lease>,
}

/// Where the block's input lease stands.
#[derive(Default)]
struct Lease {
    /// The block has the focus, so its lease is wanted.
    wanted: bool,
    held: bool,
    /// The pending `AcquireLease`.
    asking: Option<u64>,
    /// The place the window last gave the block.
    place: Option<(u16, u16)>,
    /// The size last asked for.
    sizing: Option<(u16, u16)>,
}

impl Block {
    /// Connects to `block`, attaches and starts its reader.
    fn open(
        socket: &Path,
        block: BlockId,
        place: (u16, u16),
    ) -> Result<(PaneWriter, Inbox<Feed>), String> {
        let by = Instant::now() + HANDSHAKE;
        let opened = reach(socket, block, None, by, &NEVER).map_err(|m| match m {
            Missed::Retry(why) | Missed::Broken(why, _) => why,
            Missed::Dropped => "the server let the client go".into(),
        })?;
        let shared = Arc::new(Block {
            block,
            stream: opened.stream,
            conn: Mutex::new(opened.writer),
            next: AtomicU64::new(1),
            lease: Mutex::new(Lease {
                place: Some(place),
                ..Lease::default()
            }),
        });
        shared.send(|req| Message::Attach { req, block });
        let (tx, feed) = std::sync::mpsc::sync_channel(FEED_AHEAD);
        let reader = Arc::clone(&shared);
        let conn = opened.conn;
        std::thread::Builder::new()
            .name(format!("mux-block-{block}"))
            .spawn(move || reader.read(conn, &tx))
            .map_err(|e| e.to_string())?;
        Ok((PaneWriter(shared), feed))
    }

    fn lease(&self) -> MutexGuard<'_, Lease> {
        self.lease.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn send(&self, make: impl FnOnce(u64) -> Message) -> u64 {
        let req = self.next.fetch_add(1, Ordering::Relaxed);
        let conn = &mut self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        // A failed send ends the reader too, which ends the pane.
        let _ = conn.send(&make(req));
        req
    }

    /// Asks for the lease when `wanted` and not held or asked for; gives
    /// it back when not.
    fn want_lease(&self, wanted: bool) {
        let block = self.block;
        let mut lease = self.lease();
        lease.wanted = wanted;
        match wanted {
            true if !lease.held && lease.asking.is_none() => {
                lease.asking = Some(self.send(|req| Message::AcquireLease { req, block }));
            }
            false if lease.held => {
                self.send(|req| Message::ReleaseLease { req, block });
                lease.held = false;
            }
            _ => {}
        }
    }

    /// Resizes the block to its place when the lease allows and it is not
    /// that size already asked for.
    fn fit(&self, lease: &mut Lease) {
        let Some((cols, rows)) = lease
            .place
            .filter(|p| lease.held && lease.sizing != Some(*p))
        else {
            return;
        };
        let Ok(size) = Size::new(cols, rows) else {
            return;
        };
        lease.sizing = Some((cols, rows));
        let block = self.block;
        self.send(|req| Message::Resize { req, block, size });
    }

    /// A lease answer for request `req`: `granted`, or refused (which,
    /// for any other request, means input after the lease was lost).
    fn answered(&self, req: u64, granted: bool) {
        let block = self.block;
        let mut lease = self.lease();
        if lease.asking == Some(req) {
            lease.asking = None;
            match (granted, lease.wanted) {
                (true, true) => {
                    lease.held = true;
                    self.fit(&mut lease);
                }
                // Granted after the focus moved on: given back.
                (true, false) => {
                    self.send(|req| Message::ReleaseLease { req, block });
                }
                (false, _) => {}
            }
        } else if !granted {
            lease.held = false;
        }
    }

    /// Reads the block's connection into feed steps until it ends, the
    /// block closes or the window drops the pane.
    fn read(&self, mut conn: Conn<UnixStream>, feed: &SyncSender<Feed>) {
        let block = self.block;
        let mut checkpoint = Some(Receiver::default());
        while let Ok(message) = conn.recv() {
            let step = match message {
                Message::Checkpoint {
                    block: b,
                    offset,
                    total,
                    data,
                } if b == block => {
                    let pushed = checkpoint.as_mut().map(|c| c.push(offset, total, &data));
                    if !matches!(pushed, Some(Ok(()))) {
                        eprintln!("rusty-mux-gui: block {block}: a bad checkpoint");
                        return;
                    }
                    continue;
                }
                Message::Ready { block: b } if b == block => {
                    match checkpoint.take().map(Receiver::finish) {
                        Some(Ok((grid, parser))) => Feed::Screen(Box::new(grid), Box::new(parser)),
                        _ => {
                            eprintln!("rusty-mux-gui: block {block}: a bad checkpoint");
                            return;
                        }
                    }
                }
                Message::Output { block: b, data } if b == block => Feed::Bytes(data),
                Message::Event {
                    event: Event::BlockResized { block: b, size },
                    ..
                } if b == block => {
                    self.lease().sizing = None;
                    Feed::Resized {
                        cols: size.cols(),
                        rows: size.rows(),
                    }
                }
                Message::Event {
                    event: Event::BlockClosed { block: b },
                    ..
                } if b == block => return,
                Message::Done { req, .. } => {
                    self.answered(req, true);
                    continue;
                }
                Message::Failed { req, .. } => {
                    self.answered(req, false);
                    continue;
                }
                _ => continue,
            };
            if feed.send(step).is_err() {
                return;
            }
        }
    }
}

/// The window's end of a block: typing and sizing go to the server while
/// the lease is held. Dropping it detaches the block's connection.
struct PaneWriter(Arc<Block>);

impl BackendHandle for PaneWriter {
    fn read_into(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Ok(0) // the block's output comes through the feed
    }

    /// Sends `data` to the block if it holds the lease; else drops it, as
    /// the server would refuse it.
    fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        if self.0.lease().held {
            let block = self.0.block;
            let data = data.to_vec();
            self.0.send(|req| Message::Input { req, block, data });
        }
        Ok(())
    }

    fn try_clone(&self) -> std::io::Result<Box<dyn BackendHandle>> {
        Err(std::io::Error::other("a block has one writer"))
    }

    /// Records the block's place, and asks for it when the lease allows.
    fn set_winsize(&mut self, cols: u16, rows: u16) -> std::io::Result<()> {
        let mut lease = self.0.lease();
        lease.place = Some((cols, rows));
        self.0.fit(&mut lease);
        Ok(())
    }

    #[cfg(unix)]
    fn pty_fd(&self) -> std::os::unix::io::RawFd {
        -1
    }
}

impl Drop for PaneWriter {
    /// Detached first: a server keeps a client whose connection just
    /// ends, lease and all, for a reconnect.
    fn drop(&mut self) {
        self.0.send(|req| Message::Detach { req });
        let _ = self.0.stream.shutdown(Shutdown::Both);
    }
}

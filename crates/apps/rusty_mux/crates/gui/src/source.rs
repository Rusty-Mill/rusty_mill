//! One server session as the GUI window's source.
//!
//! [`attach`] says Hello on a control connection, picks the window and
//! reads the session: its windows, their layouts and the clients. The
//! window shows one server window at a time, the session's active one, and
//! the top strip shows the rest (handoff D105). [`MuxSource`] opens a
//! connection per shown block and
//! attaches it; the window drops a pane to detach its block. The focus is
//! the window's on the server (C41): a move here is told with
//! `FocusBlock`, a move by another client comes back as `FocusChanged`.
//! Only the focused block's lease is held, so other clients can type into
//! the rest, and only a leased block is resized (pack D102). The control
//! connection also reads what the blocks' headers say ([`Headers`]).
//!
//! When the control connection ends, [`Events`] takes this client and its
//! blocks' clients over on a fresh one, telling the window how it goes
//! with [`Link`] (gui-design D62, handoff D110-D113): inside the server's
//! grace it tries on the TUI's schedule; after, or when the server let the
//! client go, it waits for the user's Retry, which attaches anew.

use std::collections::{HashMap, HashSet};
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{
    Receiver as Inbox, RecvTimeoutError, Sender, SyncSender, TrySendError, channel, sync_channel,
};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

#[cfg(windows)]
use platform_windows::WindowsUnixStream as UnixStream;
use rusty_mux_client::Conn;
use rusty_mux_client::attach::{
    CONTROL, HANDSHAKE, Missed, Opened, PANE, Plan, Requests, Welcomed, backoff, clone, connect,
    first_session_name, reach_with, take_over_with,
};
use rusty_mux_client::checkpoint::{HistoryReceiver, Receiver};
use rusty_mux_client::forward::{self, Spec, Want};
use rusty_mux_client::older::Older;
use rusty_mux_core::{
    BlockId, ClientId, Event, Layout, Selection, SessionId, Side, Size, WindowId,
};
use rusty_mux_proto::MAX_CLIPBOARD;
use rusty_mux_proto::schema::Value;
use rusty_mux_proto::{Entry, Features, Launch, Message, Program, ShowTo, Takeover};
use rusty_term::backend::BackendHandle;
use rusty_term::gui::source::{
    Act, BlockInfo, Detached, Dir, Feed, Float as Pane, ForwardDone, Link, ListDone, Listed,
    Notice, PaneIo, Remote, Session, Show, Source, Take, Tree,
};

use crate::floats;
use crate::headers::{Call, Effect, Headers};
use crate::keys::Keys;
use crate::lease::{Ask as LeaseAsk, Lease};
use crate::session::{Change, Model, StripKeys, local_host};
use crate::tree::tree;

/// What the GUI's control connection asks for: the TUI's set, plus names
/// for the strip and who each client is, to count them, and block calls
/// and the events that say a block's folder, status or foreground program
/// changed, for its header (handoff D105), and launches, for a block that
/// runs a chosen program or starts in a chosen folder. Floats (C42), for
/// blocks drawn over the layout, which the window draws and edits. A server without
/// some grants the rest, and the chrome shows less.
pub const GUI: Features = CONTROL
    .union(Features::LABELS)
    .union(Features::IDENTITY)
    .union(Features::BLOCK_CALL)
    .union(Features::STATUS)
    .union(Features::TERMINAL_EVENTS)
    .union(Features::PROCESS)
    .union(Features::MORE_PROGRAM_EVENTS)
    .union(Features::LAUNCH)
    .union(Features::FLOATS)
    .union(Features::SHOW_SESSION)
    .union(Features::AGENT_META);

/// What a block's connection asks for: the panes' features, and the input
/// gate's grants and lease requests, so an owner-only block can be taken
/// over (handoff D153), and folder listings, for the palette's directory
/// browse (D78). [`PANE`] asks for clipboard reads (C54), which the
/// window answers by its policy.
pub const BLOCK: Features = PANE
    .union(Features::INPUT_GATE)
    .union(Features::LEASE_REQUESTS)
    .union(Features::BACKGROUND_HISTORY)
    .union(Features::BLOCK_CALL)
    .union(Features::DIR_LISTING)
    .union(Features::PORT_FORWARD);

/// Never set: the handshakes here are not cancelled.
static NEVER: AtomicBool = AtomicBool::new(false);

/// Feed steps a block's reader keeps ahead of the window's parser.
const FEED_AHEAD: usize = 16;

/// How long the server keeps a client whose connection ended (handoff
/// D44, the server's `ORPHAN_GRACE`): the reconnecting banner's countdown.
const GRACE: Duration = Duration::from_secs(30);

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

/// What the blocks this client makes run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Start {
    /// The program and folder of a new session's first block. Its folder
    /// is also where a split or a new window starts when the folder of
    /// the block it goes beside isn't known.
    pub first: Launch,
    /// The program of the blocks made later, by a split or a new window;
    /// `None` is the server's default shell.
    pub shell: Option<Program>,
}

/// Connects to the server at `socket` and picks `block`'s window, or that
/// of a new session's block without one; that block takes the focus.
pub fn attach(socket: &Path, block: Option<BlockId>) -> Result<Attached, String> {
    attach_with(socket, block, Start::default())
}

/// [`attach`], with new blocks running what `start` says. `start.first`
/// is used only when there is no `block` to show.
pub fn attach_with(
    socket: &Path,
    block: Option<BlockId>,
    start: Start,
) -> Result<Attached, String> {
    let start_cwd = start.first.cwd.clone();
    let (stream, mut conn) = hello(socket)?;
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
            req.create_with(size, first_session_name(welcomed.features), start.first)?
        }
    };
    let entries = req.call(|req| Message::List { req })?;
    let follows_active = welcomed.features.contains(Features::ACTIVE_WINDOW);
    let mut model = Model::from_list(&entries, block, local_host(), follows_active)?;
    model.set_me(welcomed.client);
    model.set_knows_agents(welcomed.features.contains(Features::AGENT_META));
    let calls_blocks = welcomed.features.contains(Features::BLOCK_CALL);
    let next = req.next;
    let layout = model.view().0.cloned().unwrap_or(Layout::Block(block));
    let (asks, inbox) = std::sync::mpsc::channel();
    let state = Arc::new(State {
        socket: socket.to_owned(),
        control: Mutex::new(Conn::new(clone(&stream)?)),
        next: AtomicU64::new(next),
        tells_focus: welcomed.features.contains(Features::FOCUS),
        edits_layout: welcomed.features.contains(Features::LAYOUT_EDIT),
        selects: follows_active,
        floats: welcomed.features.contains(Features::FLOATS),
        edits: Mutex::new(HashSet::new()),
        opened: Mutex::new(HashSet::new()),
        calls_blocks,
        shows: welcomed.features.contains(Features::SHOW_SESSION),
        session: Mutex::new(model.session_id()),
        switch: Mutex::new(None),
        shown: Mutex::new(Shown::default()),
        asks: Mutex::new(asks),
        cwd: start_cwd,
        folders: Mutex::new(HashMap::new()),
        shell: start.shell,
    });
    let (first, focus) = {
        let mut shown = state.shown();
        let first = tree(&layout, &mut shown.keys);
        let focus = shown.keys.key(block);
        shown.focus = Some(focus);
        shown.window = Some(model.shown());
        // Not the stand-in `layout`: a float-only window has no tile.
        shown.tiled = model.view().0.map(Layout::blocks).unwrap_or_default();
        (first, focus)
    };
    state.tell_focus(block);
    let mut events = Events {
        state: Arc::clone(&state),
        conn,
        model,
        headers: Headers::default(),
        me: welcomed.takeover,
        asks: inbox,
        older: Older::check(GUI, welcomed.features, welcomed.version.as_deref()),
    };
    events.follow_blocks();
    events.take_headers(&entries);
    Ok(Attached {
        block,
        focus,
        source: MuxSource(state),
        first,
        events,
    })
}

/// Connects to `socket` and says the GUI's Hello, asking for no takeover.
fn hello(socket: &Path) -> Result<(UnixStream, Conn<UnixStream>), String> {
    let stream = connect(socket)?;
    let mut conn = Conn::new(clone(&stream)?);
    conn.send(&Message::Hello {
        version: Some(rusty_mux_proto::VERSION.into()),
        features: GUI,
        resume: None,
        takeover: None,
    })
    .map_err(|e| e.to_string())?;
    Ok((stream, conn))
}

/// What the user asked of a lost connection (gui-design D62).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ask {
    /// "Retry now" or "Retry".
    Retry,
    /// "Stop trying".
    Stop,
}

/// What the client and the window share.
struct State {
    socket: PathBuf,
    control: Mutex<Conn<UnixStream>>,
    next: AtomicU64,
    /// The server takes `FocusBlock` (C41).
    tells_focus: bool,
    /// The server takes `MoveBlock`, so a detached block can be put back
    /// (D61); without, none is listed.
    edits_layout: bool,
    /// The server takes `SelectWindow` and tells the active window.
    selects: bool,
    /// The server floats blocks (C42): the window draws and edits them.
    floats: bool,
    /// The requests that place a float, until they are answered: one the
    /// server refuses tells the window where the float is, so it snaps back.
    edits: Mutex<HashSet<u64>>,
    /// The `Sync`s sent after a pane opened, until answered: its client is
    /// this window's from then, which changes who a header calls another
    /// client holding input.
    opened: Mutex<HashSet<u64>>,
    /// The server answers `BlockCall`, which the headers read with.
    calls_blocks: bool,
    /// The server takes `ShowSession` (C64).
    shows: bool,
    /// The session shown, where a new window goes.
    session: Mutex<SessionId>,
    /// The session the user picked, for the events reader to show.
    switch: Mutex<Option<SessionId>>,
    shown: Mutex<Shown>,
    /// Where the window's Retry and Stop go, for the events reader.
    asks: Mutex<Sender<Ask>>,
    /// Where a split or a new window starts when its neighbour's folder
    /// isn't known.
    cwd: Option<String>,
    /// Each block's folder, as its header last read it.
    folders: Mutex<HashMap<BlockId, String>>,
    /// What a split or a new window runs.
    shell: Option<Program>,
}

/// The blocks shown and which has the focus.
#[derive(Default)]
struct Shown {
    keys: Keys,
    /// The strip's tab keys for the session's windows.
    windows: Keys<WindowId>,
    /// The switcher's keys for the server's sessions.
    sessions: Keys<SessionId>,
    /// The actions menu's keys for the account's other windows (C64).
    peers: Keys<ClientId>,
    /// Each open pane's block connection, kept weakly: the window owns it.
    blocks: HashMap<u64, std::sync::Weak<Block>>,
    focus: Option<u64>,
    /// The server window shown.
    window: Option<WindowId>,
    /// The shown window's tiled blocks: a float or a detached block is
    /// none, and the server takes only a tile to place a block beside.
    tiled: Vec<BlockId>,
}

/// The block a reattached one goes beside: the `focused` block if it is a
/// tile, else the first tile, else none, which the server fills an empty or
/// float-only window for (C42).
fn anchor(tiled: &[BlockId], focused: Option<BlockId>) -> Option<BlockId> {
    focused
        .filter(|b| tiled.contains(b))
        .or_else(|| tiled.first().copied())
}

impl State {
    fn shown(&self) -> MutexGuard<'_, Shown> {
        lock(&self.shown)
    }

    /// What a split or a new window beside `block` runs, and where: in
    /// `block`'s folder when it is known and is one here (the server
    /// refuses any other), else in the start folder.
    fn launch(&self, block: Option<BlockId>) -> Launch {
        let folder = block
            .and_then(|b| lock(&self.folders).get(&b).cloned())
            .filter(|dir| Path::new(dir).is_dir());
        Launch {
            program: self.shell.clone(),
            cwd: folder.or_else(|| self.cwd.clone()),
            ..Launch::default()
        }
    }

    /// Tells the server the window's focus moved to `block`.
    fn tell_focus(&self, block: BlockId) {
        if self.tells_focus {
            self.send(|req| Message::FocusBlock { req, block });
        }
    }

    /// The clients this window's block connections are.
    fn mine(&self) -> Vec<ClientId> {
        self.open_blocks().iter().map(|b| b.client).collect()
    }

    /// The blocks the window has open.
    fn open_blocks(&self) -> Vec<Arc<Block>> {
        let shown = self.shown();
        shown
            .blocks
            .values()
            .filter_map(std::sync::Weak::upgrade)
            .collect()
    }

    /// Passes the user's `ask` to the events reader.
    fn ask(&self, ask: Ask) {
        // No reader: the window is closing anyway.
        let _ = lock(&self.asks).send(ask);
    }

    /// The server floats blocks and lets this client edit the layout: the
    /// window may move them.
    fn edits_floats(&self) -> bool {
        self.floats && self.edits_layout
    }

    /// A place the window waits on that goes nowhere: tells the window where
    /// its floats are by an answered no-op, so the one dropped there snaps
    /// back instead of staying where it was let go.
    fn floats_told_again(&self, act: &Act) {
        if matches!(act, Act::Place { .. }) {
            self.send(|req| {
                lock(&self.edits).insert(req);
                Message::Sync { req }
            });
        }
    }

    /// Sends a request on the control connection, returning its number;
    /// the events reader skips its answer unless it asked for it.
    fn send(&self, make: impl FnOnce(u64) -> Message) -> u64 {
        let req = self.next.fetch_add(1, Ordering::Relaxed);
        let control = &mut self.control.lock().unwrap_or_else(PoisonError::into_inner);
        // A dead control connection ends the events reader, which closes
        // the window.
        let _ = control.send(&make(req));
        req
    }

    /// Sends the request `make` builds, if it builds one.
    fn try_send(&self, make: impl FnOnce(u64) -> Option<Message>) {
        let req = self.next.fetch_add(1, Ordering::Relaxed);
        if let Some(message) = make(req) {
            let control = &mut self.control.lock().unwrap_or_else(PoisonError::into_inner);
            // A dead control connection ends the events reader.
            let _ = control.send(&message);
        }
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

/// `m`'s guard, whether or not a holder panicked: every value here stays
/// whole between statements.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The server window's blocks, opened on demand by the GUI window.
pub struct MuxSource(Arc<State>);

impl MuxSource {
    /// Pane `key`'s block connection, while the window has it open.
    fn pane(&self, key: u64) -> Option<Arc<Block>> {
        self.0
            .shown()
            .blocks
            .get(&key)
            .and_then(std::sync::Weak::upgrade)
    }

    /// Carries out the user's float `act` on pane `key`'s block (C42,
    /// gui-design D91): the window follows the `FloatsChanged` it causes. A
    /// place the server refuses is told back, so the float snaps to where it is.
    fn float(&self, key: u64, act: Act) {
        let block = self.0.shown().keys.id(key);
        let (true, Some(block)) = (self.0.edits_floats(), block) else {
            return self.0.floats_told_again(&act);
        };
        match act {
            Act::Float(_) => {
                if let Some(at) = floats::first() {
                    self.0.send(|req| Message::PlaceFloat { req, block, at });
                }
            }
            Act::Place { at, .. } => {
                match floats::rect(at) {
                    // Told before it is sent: a refusal can come back before
                    // `send` does, and one not found here would be lost.
                    Some(at) => {
                        self.0.send(|req| {
                            lock(&self.0.edits).insert(req);
                            Message::PlaceFloat { req, block, at }
                        });
                    }
                    None => self.0.floats_told_again(&act),
                }
            }
            Act::Stack { top, .. } => {
                self.0.send(|req| Message::StackFloat { req, block, top });
            }
            _ => {}
        }
    }

    /// Carries out the user's take-over `act` on pane `key`'s block
    /// (gui-design D64, handoff D153).
    fn take(&self, key: u64, act: Act) {
        let block = self
            .0
            .shown()
            .blocks
            .get(&key)
            .and_then(std::sync::Weak::upgrade);
        let Some(block) = block else {
            return;
        };
        block.update(|lease, send| match act {
            Act::TakeOver(_) => lease.take_over(send),
            Act::CancelAsk(_) => lease.cancel_ask(),
            Act::GiveBack(_) => lease.give_back(send),
            _ => {}
        });
    }
}

impl Source for MuxSource {
    fn open(&self, key: u64, cols: u16, rows: u16) -> std::io::Result<PaneIo> {
        let block = self
            .0
            .shown()
            .keys
            .id(key)
            .ok_or_else(|| std::io::Error::other(format!("no block has key {key}")))?;
        let (pane, feed) =
            Block::open(&self.0.socket, block, (cols, rows)).map_err(std::io::Error::other)?;
        let mut shown = self.0.shown();
        shown.blocks.retain(|_, b| b.strong_count() > 0);
        shown.blocks.insert(key, Arc::downgrade(&pane.0));
        pane.0.want_lease(shown.focus == Some(key));
        let serves_clipboard = pane.0.clipboard;
        drop(shown);
        // A lease event may have come before the pane was known as this
        // window's: its headers are read again once everything sent so far is
        // answered.
        // Recorded before it is sent: the answer can come back first.
        self.0.send(|req| {
            lock(&self.0.opened).insert(req);
            Message::Sync { req }
        });
        Ok(PaneIo {
            writer: Box::new(pane),
            feed: Box::new(feed.into_iter()),
            // The server's own terminal answers the program's queries.
            answers_queries: false,
            serves_clipboard,
        })
    }

    fn floating(&self) -> bool {
        self.0.floats
    }

    fn lists(&self, key: u64) -> bool {
        self.pane(key).is_some_and(|b| b.lists)
    }

    fn list_dir(&self, key: u64, path: Option<String>, done: ListDone) {
        match self.pane(key) {
            Some(block) => block.list_dir(path, done),
            None => done(Err("the block is not open".into())),
        }
    }

    fn forwards(&self, key: u64) -> bool {
        self.pane(key).is_some_and(|b| b.forwards)
    }

    fn forward(&self, key: u64, port: u16, done: ForwardDone) {
        match self.pane(key) {
            Some(block) => block.forward(port, done),
            None => done(Err("the block is not open".into())),
        }
    }

    fn focus(&self, key: u64) {
        let block = self.0.shown().keys.id(key);
        if let Some(block) = block {
            self.0.tell_focus(block);
            self.0.focus(key);
        }
    }

    /// Makes window `key` the session's active one; the window follows
    /// when the server says so, as it does for another client's pick.
    fn select(&self, key: u64) {
        let window = self.0.shown().windows.id(key);
        if let (Some(window), true) = (window, self.0.selects) {
            self.0.send(|req| Message::SelectWindow { req, window });
        }
    }

    /// Sends a layout change (handoff D104); the window follows the
    /// `LayoutChanged` it causes. A pane not shown is left alone.
    fn act(&self, act: Act) {
        if let Act::TakeOver(key) | Act::CancelAsk(key) | Act::GiveBack(key) = act {
            return self.take(key, act);
        }
        if let Act::Float(key) | Act::Place { key, .. } | Act::Stack { key, .. } = act {
            return self.float(key, act);
        }
        let (block, window, focused, tiled) = {
            let shown = self.0.shown();
            let key = match act {
                Act::Split { key, .. } | Act::Zoom(key) | Act::Close(key) | Act::Reattach(key) => {
                    Some(key)
                }
                Act::Unzoom
                | Act::TakeOver(_)
                | Act::CancelAsk(_)
                | Act::GiveBack(_)
                | Act::Float(_)
                | Act::Place { .. }
                | Act::Stack { .. } => None,
            };
            let focused = shown.focus.and_then(|k| shown.keys.id(k));
            (
                key.and_then(|k| shown.keys.id(k)),
                shown.window,
                focused,
                shown.tiled.clone(),
            )
        };
        self.0.try_send(|req| match act {
            Act::Split { dir, .. } => block.map(|block| Message::Split {
                req,
                block,
                side: match dir {
                    Dir::Vertical => Side::Right,
                    Dir::Horizontal => Side::Down,
                },
                launch: self.0.launch(Some(block)),
            }),
            Act::Zoom(_) => block.map(|block| Message::Zoom { req, block }),
            Act::Close(_) => block.map(|block| Message::CloseBlock { req, block }),
            Act::Unzoom => window.map(|window| Message::Unzoom { req, window }),
            // Beside the focused tile, else the first; the server fills an
            // empty window.
            Act::Reattach(_) => block.map(|block| Message::MoveBlock {
                req,
                block,
                beside: anchor(&tiled, focused),
                side: Side::Right,
            }),
            Act::TakeOver(_)
            | Act::CancelAsk(_)
            | Act::GiveBack(_)
            | Act::Float(_)
            | Act::Place { .. }
            | Act::Stack { .. } => None,
        });
    }

    /// Shows session `key` (handoff D109): the events reader re-points the
    /// window at it, woken by the answer to a List sent here. No
    /// `SelectWindow`: the pick changes only what this client shows.
    fn switch(&self, key: u64) {
        let Some(session) = self.0.shown().sessions.id(key) else {
            return;
        };
        *lock(&self.0.switch) = Some(session);
        self.0.send(|req| Message::List { req });
    }

    fn retry(&self) {
        self.0.ask(Ask::Retry);
    }

    fn stop(&self) {
        self.0.ask(Ask::Stop);
    }

    /// Asks the other windows to show session `key` (C64).
    fn show_in(&self, key: u64, to: Show) -> bool {
        if !self.0.shows {
            return false;
        }
        let (session, to) = {
            let shown = self.0.shown();
            let to = match to {
                Show::All => Some(ShowTo::Others),
                Show::Window(peer) => shown.peers.id(peer).map(ShowTo::Client),
            };
            (shown.sessions.id(key), to)
        };
        let (Some(session), Some(to)) = (session, to) else {
            return false;
        };
        self.0.send(|req| Message::ShowSession { req, session, to });
        true
    }

    /// Opens session `key` in a window of its own: another `rusty-mux-gui`
    /// on that session's first block (C64).
    fn open_window(&self, key: u64) -> bool {
        let Some(session) = self.0.shown().sessions.id(key) else {
            return false;
        };
        let socket = self.0.socket.clone();
        std::thread::Builder::new()
            .name("mux-open-window".into())
            .spawn(move || {
                if let Err(e) = open_session_window(&socket, session) {
                    eprintln!("rusty-mux-gui: open in new window: {e}");
                }
            })
            .map_err(|e| eprintln!("rusty-mux-gui: open in new window: {e}"))
            .is_ok()
    }

    /// Creates a window with one block and selects it, on a connection of
    /// its own so the UI never waits for the server.
    fn new_window(&self) {
        let (socket, session) = (self.0.socket.clone(), *lock(&self.0.session));
        let focused = {
            let shown = self.0.shown();
            shown.focus.and_then(|k| shown.keys.id(k))
        };
        let launch = self.0.launch(focused);
        let spawned = std::thread::Builder::new()
            .name("mux-new-window".into())
            .spawn(move || {
                if let Err(e) = create_window(&socket, session, launch) {
                    eprintln!("rusty-mux-gui: new window: {e}");
                }
            });
        if let Err(e) = spawned {
            eprintln!("rusty-mux-gui: new window: {e}");
        }
    }
}

/// Starts a `rusty-mux-gui` on the first block of `session`'s first window
/// (C64), with the options this one runs with.
fn open_session_window(socket: &Path, session: SessionId) -> Result<(), String> {
    let stream = connect(socket)?;
    let mut conn = Conn::new(stream);
    conn.send(&Message::Hello {
        version: Some(rusty_mux_proto::VERSION.into()),
        features: Features::CHECKPOINT_V6.union(Features::INSPECT),
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
    req.welcome()?;
    let entries = req.call(|req| Message::List { req })?;
    let block = first_block(&entries, session).ok_or("the session has no block to show")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .args(forwarded(std::env::args_os().skip(1)))
        .arg(socket)
        .arg(block.to_string())
        .spawn()
        .map(|mut child| {
            // Reaped when it ends, so it never lingers as a zombie.
            std::thread::spawn(move || child.wait());
        })
        .map_err(|e| e.to_string())
}

/// A laid-out block of `session` to show: the active window's first, else the
/// first window with one, in the order listed. A detached block is not one:
/// its window has no layout (a server that can ask for this always sends it).
/// Nor is a float: the new window does not ask for floats (C42), so a session
/// with only floating blocks has none to show, and the ask is refused.
fn first_block(entries: &[Message], session: SessionId) -> Option<BlockId> {
    let entries = || {
        entries.iter().filter_map(|m| match m {
            Message::Entry { entry, .. } => Some(entry),
            _ => None,
        })
    };
    let active = entries().find_map(|e| match e {
        Entry::Session {
            session: s, active, ..
        } if *s == session => *active,
        _ => None,
    });
    let windows = entries().filter_map(|e| match e {
        Entry::Window {
            window, session: s, ..
        } if *s == session => Some(*window),
        _ => None,
    });
    let in_window = |window: WindowId| {
        entries().find_map(|e| match e {
            Entry::Window {
                window: w,
                layout: Some(layout),
                ..
            } if *w == window => layout.blocks().first().copied(),
            _ => None,
        })
    };
    active.into_iter().chain(windows).find_map(in_window)
}

/// The options a new window runs with, as this one was started: the
/// renderer, the config file and the profile.
fn forwarded(args: impl Iterator<Item = std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    let mut out = Vec::new();
    let mut args = args;
    while let Some(arg) = args.next() {
        let Some(text) = arg.to_str() else {
            continue;
        };
        match text {
            "--" => break,
            "--gpu" => out.push(arg),
            // Options this window keeps to itself still take their value.
            "--cwd" => {
                args.next();
            }
            "--config" | "--profile" => {
                out.push(arg);
                out.extend(args.next());
            }
            _ if text.starts_with("--config=") || text.starts_with("--profile=") => out.push(arg),
            _ => {}
        }
    }
    out
}

/// Creates a window in `session` with one block, and makes it active.
/// The connection asks for no takeover, so the server forgets it at once;
/// the handshake needs checkpoints.
fn create_window(socket: &Path, session: SessionId, launch: Launch) -> Result<(), String> {
    let stream = connect(socket)?;
    let mut conn = Conn::new(stream);
    conn.send(&Message::Hello {
        version: Some(rusty_mux_proto::VERSION.into()),
        features: Features::CHECKPOINT_V6
            .union(Features::ACTIVE_WINDOW)
            .union(Features::LAUNCH),
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
    req.welcome()?;
    let window = WindowId::from_uuid(req.create_id(|req| Message::CreateWindow {
        req,
        session,
        name: String::new(),
    })?);
    let size = Size::new(FIRST_SIZE.0, FIRST_SIZE.1).map_err(|e| e.to_string())?;
    req.create_id(|req| Message::CreateBlock {
        req,
        window,
        size,
        launch,
    })?;
    req.call(|req| Message::SelectWindow { req, window })?;
    Ok(())
}

/// What the control connection tells the window.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// Show this layout with these floats over it, back to front; `None`
    /// closes the window, and comes last.
    Layout(Option<Tree>, Vec<Pane>),
    /// Only the floats changed (C42): every float of the shown window.
    Floats(Vec<Pane>),
    /// Another client moved the focus to this pane.
    Focus(u64),
    /// Show this pane alone, or every pane with `None`; follows the
    /// layout it applies to.
    Zoom(Option<u64>),
    /// Show no pane: every block of the window is detached.
    Empty,
    /// The session changed, for the top strip.
    Session(Session),
    /// What the shown blocks' headers say, by pane key.
    Blocks(Vec<(u64, BlockInfo)>),
    /// How the connection to the server stands (gui-design D62).
    Link(Link),
    /// The info notice for the banner, `None` for none (gui-design D63).
    Notice(Option<Notice>),
    /// The shown session, named, ended while the connection was away:
    /// show no block until another is picked (handoff D114).
    Ended(String),
    /// Another window, named, asked this one to show the session with this
    /// key (C64).
    Shown(u64, String),
}

/// Why the events reader stopped following the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    /// The control connection ended.
    Lost,
    /// The session is gone: the window closes.
    Gone,
    /// The window is gone.
    Closed,
}

/// The control connection's events for the window.
pub struct Events {
    state: Arc<State>,
    conn: Conn<UnixStream>,
    model: Model,
    headers: Headers,
    /// How to take the control connection's client over, while the
    /// server keeps it.
    me: Option<Takeover>,
    asks: Inbox<Ask>,
    /// What the server lacks of what this client asked for, said once per
    /// connection (C36).
    older: Option<Older>,
}

impl Events {
    /// Follows the session on a thread of its own, telling `remote`; the
    /// session's end, or the connection's, closes the window.
    pub fn follow(self, remote: Remote) {
        std::thread::spawn(move || {
            self.run(|out| match out {
                Out::Layout(t, floats) => remote.layout(t, floats),
                Out::Floats(floats) => remote.floats(floats),
                Out::Focus(k) => remote.focus(k),
                Out::Zoom(k) => remote.zoom(k),
                Out::Empty => remote.empty(),
                Out::Session(s) => remote.session(s),
                Out::Blocks(b) => remote.blocks(b),
                Out::Link(l) => remote.link(l),
                Out::Notice(n) => remote.notice(n),
                Out::Ended(name) => remote.ended(name),
                Out::Shown(key, by) => remote.shown(key, by),
            })
        });
    }

    /// Follows the session until it ends or `out` says `false`, bringing
    /// the connection back when it ends. The session goes out first, then
    /// every change.
    pub fn run(mut self, mut out: impl FnMut(Out) -> bool) {
        if !(out(Out::Session(self.session()))
            && out(Out::Floats(self.floats()))
            && out(Out::Zoom(self.zoomed()))
            && out(Out::Notice(self.notice())))
        {
            return;
        }
        let stop = loop {
            let stop = match self.follow_session(&mut out) {
                Stop::Lost => self.reconnect(&mut out),
                stop => Some(stop),
            };
            if let Some(stop) = stop {
                break stop;
            }
        };
        if stop == Stop::Gone {
            out(Out::Layout(None, Vec::new()));
        }
    }

    /// Follows the session until the connection or the session ends, or
    /// `out` says `false`.
    fn follow_session(&mut self, out: &mut impl FnMut(Out) -> bool) -> Stop {
        while let Ok(message) = self.conn.recv() {
            if !self.headed(&message, out) {
                return Stop::Closed;
            }
            if let Message::SessionShown { session, by } = &message {
                let key = self.state.shown().sessions.key(*session);
                if !out(Out::Shown(key, self.model.window_name(*by))) {
                    return Stop::Closed;
                }
            }
            // A sync that fails is answered too, and forgotten.
            if let Message::Failed { req, .. } = &message {
                lock(&self.state.opened).remove(req);
            }
            if let Message::Done { req, .. } = &message
                && lock(&self.state.opened).remove(req)
                && !out(Out::Blocks(self.blocks()))
            {
                return Stop::Closed;
            }
            // A place the window waits on is answered with where its floats
            // are: a refusal, or an answer with no change to follow, leaves
            // the one dropped there where the server has it (D91).
            if let Message::Failed { req, .. } | Message::Done { req, .. } = &message
                && lock(&self.state.edits).remove(req)
                && !out(Out::Floats(self.floats()))
            {
                return Stop::Closed;
            }
            let change = match &message {
                Message::Event { event, .. } => self.model.apply(event),
                Message::Entry { entry, .. } => self.model.take(entry),
                _ => Change::None,
            };
            if !self.switched(out) {
                return Stop::Closed;
            }
            if let Message::Event {
                event:
                    Event::WindowCreated { .. }
                    | Event::SessionCreated { .. }
                    | Event::ClientConnected { .. }
                    | Event::BlockCreated { .. },
                ..
            } = message
            {
                // Its name, what a client can do, or who works a new block
                // (the overview's `Entry::Agent`, D97) comes with its entry.
                self.state.send(|req| Message::List { req });
            }
            if matches!(change, Change::Layout | Change::Shown) {
                self.state.shown().window = Some(self.model.shown());
            }
            if matches!(change, Change::Layout | Change::Shown | Change::Strip) {
                self.follow_blocks();
            }
            let go_on = match change {
                Change::None => true,
                Change::Strip => out(Out::Session(self.session())),
                Change::Holders => out(Out::Blocks(self.blocks())),
                // The strip follows: a block that goes between a tile, a float
                // and the detached list changes its thumbnail and that list.
                Change::Floats => {
                    out(Out::Floats(self.floats())) && out(Out::Session(self.session()))
                }
                // The strip follows: the thumbnail and detached blocks.
                Change::Layout => self.layout(out) && out(Out::Session(self.session())),
                Change::Focus(block) => {
                    let key = self.state.shown().keys.key(block);
                    self.state.focus(key);
                    out(Out::Focus(key))
                }
                Change::Shown => {
                    self.layout(out) && self.focus_shown(out) && out(Out::Session(self.session()))
                }
                Change::Emptied(window) => {
                    self.state.send(|req| Message::SelectWindow { req, window });
                    out(Out::Session(self.session()))
                }
                Change::Gone => return Stop::Gone,
            };
            if !go_on {
                return Stop::Closed;
            }
        }
        Stop::Lost
    }

    /// Brings the lost control connection back (handoff D110, D113):
    /// inside the grace it takes the clients over on the TUI's schedule,
    /// at once on "Retry now"; past it, after "Stop trying" or once the
    /// server let the client go, it waits for "Retry". `None` once back.
    fn reconnect(&mut self, out: &mut impl FnMut(Out) -> bool) -> Option<Stop> {
        // Asks from before the loss were for another one.
        while self.asks.try_recv().is_ok() {}
        let lost = Instant::now();
        let mut tries = 0;
        let mut at = lost;
        while self.me.is_some() {
            let left = GRACE.saturating_sub(lost.elapsed());
            if left.is_zero() {
                break;
            }
            let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
            let link = |trying| {
                Out::Link(Link::Reconnecting {
                    left: secs as u32,
                    trying,
                })
            };
            if !out(link(false)) {
                return Some(Stop::Closed);
            }
            // Wake as the countdown ticks, or for the next try.
            let tick = left.saturating_sub(Duration::from_secs(secs - 1));
            let wait = at.saturating_duration_since(Instant::now()).min(tick);
            match self.asks.recv_timeout(wait) {
                Ok(Ask::Retry) => at = Instant::now(),
                Ok(Ask::Stop) => break,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Some(Stop::Closed),
            }
            if Instant::now() < at {
                continue;
            }
            if !out(link(true)) {
                return Some(Stop::Closed);
            }
            match self.take_over() {
                Ok(()) => return self.back(true, out),
                Err(Missed::Dropped) => self.me = None,
                Err(Missed::Retry(_)) => {}
                Err(Missed::Broken(_, me)) => self.me = Some(me),
            }
            tries += 1;
            at = Instant::now() + backoff(tries);
        }
        self.down(out)
    }

    /// The error banner: waits for "Retry", which takes the clients over
    /// while the server may keep them, else attaches anew (D113).
    fn down(&mut self, out: &mut impl FnMut(Out) -> bool) -> Option<Stop> {
        loop {
            if !out(Out::Link(Link::Down { trying: false })) {
                return Some(Stop::Closed);
            }
            match self.asks.recv() {
                Ok(Ask::Retry) => {}
                Ok(Ask::Stop) => continue,
                Err(_) => return Some(Stop::Closed),
            }
            if !out(Out::Link(Link::Down { trying: true })) {
                return Some(Stop::Closed);
            }
            if self.me.is_some() {
                match self.take_over() {
                    Ok(()) => return self.back(true, out),
                    Err(Missed::Dropped) => self.me = None,
                    Err(Missed::Retry(_)) => continue,
                    Err(Missed::Broken(_, me)) => {
                        self.me = Some(me);
                        continue;
                    }
                }
            }
            match self.attach_anew() {
                Ok(()) => return self.back(false, out),
                Err(why) => eprintln!("rusty-mux-gui: reconnecting: {why}"),
            }
        }
    }

    /// Takes this client and its open blocks' clients over on a fresh
    /// connection (handoff D111), hands each block its connection back
    /// and reads the session again. `Some(Gone)` when the session ended
    /// meanwhile.
    fn take_over(&mut self) -> Result<(), Missed> {
        let me = self.me.ok_or(Missed::Dropped)?;
        let blocks: Vec<(Arc<Block>, Takeover)> = self
            .state
            .open_blocks()
            .into_iter()
            .filter_map(|b| {
                let token = b.takeover()?;
                Some((b, token))
            })
            .collect();
        let plan = Plan {
            me,
            next: self.state.next.load(Ordering::Relaxed),
            window: None,
            resumes: blocks.iter().map(|(b, t)| (b.block, *t)).collect(),
        };
        let mut back = take_over_with(self.state.socket.clone(), plan, GUI, BLOCK, &NEVER)?;
        let mut req = Requests {
            conn: &mut back.conn,
            next: back.next,
            by: Some(Instant::now() + HANDSHAKE),
            cancel: &NEVER,
        };
        let entries = req
            .call(|req| Message::List { req })
            .map_err(|why| Missed::Broken(why, back.me))?;
        let next = req.next;
        for ((block, _), opened) in blocks.iter().zip(back.panes) {
            block.revive(opened);
        }
        self.install((back.conn, back.writer), Some(back.me), next, &entries);
        Ok(())
    }

    /// Attaches anew, as a new client: the window opens its blocks again,
    /// their leases from before released (D113).
    fn attach_anew(&mut self) -> Result<(), String> {
        let (stream, mut conn) = hello(&self.state.socket)?;
        let mut req = Requests {
            conn: &mut conn,
            next: 1,
            by: Some(Instant::now() + HANDSHAKE),
            cancel: &NEVER,
        };
        let Welcomed {
            takeover,
            features,
            version,
            client,
        } = req.welcome()?;
        // A new connection, perhaps to an upgraded server: told afresh.
        self.older = Older::check(GUI, features, version.as_deref());
        let entries = req.call(|req| Message::List { req })?;
        let next = req.next;
        let writer = Conn::new(clone(&stream)?);
        self.install((conn, writer), takeover, next, &entries);
        // A new client: the old one is no window of this one.
        self.model.set_me(client);
        Ok(())
    }

    /// Follows the session on the connection `(conn, writer)` from now,
    /// as its List `entries` say, the shown session ended if they lack it
    /// (handoff D114).
    fn install(
        &mut self,
        (conn, writer): (Conn<UnixStream>, Conn<UnixStream>),
        me: Option<Takeover>,
        next: u64,
        entries: &[Message],
    ) {
        self.model = self.model.relisted(entries);
        self.conn = conn;
        self.me = me;
        *lock(&self.state.control) = writer;
        self.state.next.store(next, Ordering::Relaxed);
        *lock(&self.state.session) = self.model.session_id();
        self.state.shown().window = Some(self.model.shown());
        self.headers = Headers::default();
        self.follow_blocks();
        self.take_headers(entries);
    }

    /// Says the connection is back, `kept` when its clients were taken
    /// over, then shows the session as it is now, or that it ended while
    /// the connection was away (gui-design D62, handoff D114).
    fn back(&mut self, kept: bool, out: &mut impl FnMut(Out) -> bool) -> Option<Stop> {
        if let Some(name) = self.model.ended() {
            let told = out(Out::Link(Link::Up))
                && (kept || out(Out::Notice(self.notice())))
                && out(Out::Ended(name.to_owned()))
                && out(Out::Session(self.session()));
            return (!told).then_some(Stop::Closed);
        }
        let shown = out(Out::Link(Link::Back { kept }))
            && (kept || out(Out::Notice(self.notice())))
            && self.layout(out)
            && self.focus_shown(out)
            && out(Out::Session(self.session()))
            && out(Out::Blocks(self.blocks()));
        (!shown).then_some(Stop::Closed)
    }

    /// Takes what a List's `entries` say of the blocks' headers.
    fn take_headers(&mut self, entries: &[Message]) {
        for m in entries {
            if let Message::Entry { entry, .. } = m {
                self.headers.take(entry);
            }
        }
    }

    /// Shows the session the user picked, if any: its window's layout,
    /// focus and strip, as when another window is shown. `false` once the
    /// window is gone.
    fn switched(&mut self, out: &mut impl FnMut(Out) -> bool) -> bool {
        let Some(session) = lock(&self.state.switch).take() else {
            return true;
        };
        if !self.model.switch(session) {
            return true;
        }
        *lock(&self.state.session) = session;
        self.state.shown().window = Some(self.model.shown());
        self.follow_blocks();
        self.layout(out) && self.focus_shown(out) && out(Out::Session(self.session()))
    }

    /// Follows the session's blocks for their headers and thumbnails:
    /// reads what is not known of new ones, and their lease holders from a
    /// List.
    fn follow_blocks(&mut self) {
        if !self.state.calls_blocks {
            return;
        }
        let calls = self.headers.track(&self.model.session_blocks());
        if calls.is_empty() {
            return;
        }
        for (block, call) in calls {
            self.ask(block, call);
        }
        self.state.send(|req| Message::List { req });
    }

    fn ask(&mut self, block: BlockId, call: Call) {
        let req = self.state.send(|req| Message::BlockCall {
            req,
            block,
            method: call.method().into(),
            args: Vec::new(),
        });
        self.headers.asked(req, block, call);
    }

    /// Takes what `message` says of the headers, and sends them when they
    /// changed; `false` once the window is gone.
    fn headed(&mut self, message: &Message, out: &mut impl FnMut(Out) -> bool) -> bool {
        let changed = match message {
            Message::Text { req, text } if self.headers.owns(*req) => {
                self.headers.answer(*req, Some(text))
            }
            Message::Failed { req, .. } if self.headers.owns(*req) => {
                self.headers.answer(*req, None)
            }
            Message::Entry { entry, .. } => self.headers.take(entry),
            Message::Event { event, .. } => match self.headers.apply(event) {
                Effect::None => false,
                Effect::Changed => true,
                Effect::Ask(block, call) => {
                    self.ask(block, call);
                    false
                }
            },
            _ => false,
        };
        if changed {
            *lock(&self.state.folders) = self.headers.folders();
        }
        !changed || out(Out::Blocks(self.blocks()))
    }

    /// What the shown blocks' headers say.
    fn blocks(&self) -> Vec<(u64, BlockInfo)> {
        let mine = self.state.mine();
        let keys = &mut self.state.shown().keys;
        self.headers
            .infos(keys, &mine, |c| self.model.client_name(c))
    }

    /// The banner's notice for a server older than this client.
    fn notice(&self) -> Option<Notice> {
        let older = self.older.as_ref()?;
        let host = self.session().host;
        Some(Notice {
            title: older.title(&host),
            detail: older.detail(),
        })
    }

    /// The session as the strip shows it.
    fn session(&self) -> Session {
        let shown = &mut *self.state.shown();
        let mut session = self.model.session(&mut StripKeys {
            peers: &mut shown.peers,
            sessions: &mut shown.sessions,
            windows: &mut shown.windows,
            blocks: &mut shown.keys,
        });
        if self.state.edits_layout {
            session.detached = self
                .model
                .detached()
                .into_iter()
                .map(|d| Detached {
                    key: shown.keys.key(d.block),
                    name: Some(d.name)
                        .filter(|n| !n.is_empty())
                        .or_else(|| self.headers.program(d.block))
                        .unwrap_or_else(|| "Block".into()),
                    since: d.since,
                })
                .collect();
        }
        session
    }

    /// Sends the shown window's layout and zoom; a window with no blocks
    /// ends the GUI window, unless its blocks are detached (D61).
    fn layout(&self, out: &mut impl FnMut(Out) -> bool) -> bool {
        let (layout, focused) = self.model.view();
        let floats = self.floats();
        let tiled = layout.map(|l| tree(l, &mut self.state.shown().keys));
        self.keep_focus_in(self.model.shown_blocks(), focused);
        self.state.shown().tiled = layout.map(Layout::blocks).unwrap_or_default();
        // A window of floats alone is alive: the first float stands for the
        // tile under them, which draws nothing, as it floats (C42).
        let shown = tiled.or_else(|| floats.first().map(|f| Tree::Pane(f.key)));
        if shown.is_none() && !self.model.detached().is_empty() {
            return out(Out::Empty);
        }
        let ended = shown.is_none();
        // With the layout, so a block that goes from a tile to a float keeps
        // its pane.
        out(Out::Layout(shown, floats)) && !ended && out(Out::Zoom(self.zoomed()))
    }

    /// Keeps the focus Reattach places beside on a block `laid_out`:
    /// the server's `focused` one, else the first (D61). A block just
    /// detached can't be.
    fn keep_focus_in(&self, laid_out: Vec<BlockId>, focused: Option<BlockId>) {
        let shown = &mut *self.state.shown();
        let kept = shown
            .focus
            .and_then(|k| shown.keys.id(k))
            .is_some_and(|b| laid_out.contains(&b));
        if kept {
            return;
        }
        let block = focused
            .filter(|b| laid_out.contains(b))
            .or_else(|| laid_out.first().copied());
        shown.focus = block.map(|b| shown.keys.key(b));
    }

    /// The shown window's floats as the window knows them; none from a
    /// server that does not float.
    fn floats(&self) -> Vec<Pane> {
        match self.state.floats {
            true => floats::panes(&self.model.floats(), &mut self.state.shown().keys),
            false => Vec::new(),
        }
    }

    /// The pane the shown window shows alone, if any.
    fn zoomed(&self) -> Option<u64> {
        let block = self.model.zoomed()?;
        Some(self.state.shown().keys.key(block))
    }

    /// Moves the focus to the shown window's focused block.
    fn focus_shown(&self, out: &mut impl FnMut(Out) -> bool) -> bool {
        let Some(block) = self.model.view().1 else {
            return true;
        };
        let key = self.state.shown().keys.key(block);
        self.state.focus(key);
        out(Out::Focus(key))
    }
}

/// One block's connection, shared by the pane's writer and its reader.
/// A lost connection is handed back by [`Block::revive`], the client the
/// same.
struct Block {
    block: BlockId,
    /// The client the server made of this connection.
    client: ClientId,
    stream: Mutex<UnixStream>,
    conn: Mutex<Conn<UnixStream>>,
    /// How to take the client over, if the server allows it.
    takeover: Mutex<Option<Takeover>>,
    /// Where a revived connection goes to the reader; taken when the
    /// pane is dropped, which ends the reader.
    revive: Mutex<Option<Sender<Conn<UnixStream>>>>,
    next: AtomicU64,
    lease: Mutex<Lease>,
    /// The server takes lease requests on this connection, so the window
    /// is told where taking the block over stands (D153).
    requests: bool,
    /// Where the window hears of it, while the reader runs, and what it
    /// heard last.
    to_window: Mutex<Option<SyncSender<Feed>>>,
    told: Mutex<Option<Take>>,
    /// The server lists folders on this connection (`terminal.list-dir`).
    lists: bool,
    /// The server asks this connection's client for clipboard reads
    /// (C54), so the window answers them, not the pane's own grid.
    clipboard: bool,
    /// Who waits for a listing, by request number.
    listings: Mutex<HashMap<u64, ListDone>>,
    /// The server tunnels ports on this connection (C81).
    forwards: bool,
    /// The run that tunnels this pane's ports, once one was asked for.
    forwarder: Mutex<Option<Forwarder>>,
}

/// Request numbers of a pane's forwards start here, apart from
/// [`Block::next`]'s.
const FORWARD_REQ: u64 = 1 << 40;

/// Frames queued for a pane's forward run, and asks queued for it.
const FORWARD_FRAMES: usize = 1024;
const FORWARD_WANTS: usize = 16;

/// How long a port may take to be listened on.
const LISTEN_WAIT: Duration = Duration::from_secs(10);

/// A pane's running [`forward::run_fed`], and what it listens on.
struct Forwarder {
    /// What the reader passes it of the server's frames; bounded, so a run
    /// that stalls never makes the reader wait or the queue grow.
    frames: SyncSender<Message>,
    wants: SyncSender<Want>,
    stop: Arc<AtomicBool>,
    /// The local port of each remote port listened on.
    ports: HashMap<u16, u16>,
}

/// Writes frames to a block's connection, whole, under its lock, so they
/// never interleave with the pane's own.
struct ConnWriter(Arc<Block>, Arc<AtomicBool>);

impl std::io::Write for ConnWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // A run that was stopped must not write through the connection that
        // replaced the one it served.
        if self.1.load(Ordering::SeqCst) {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        lock(&self.0.conn).write_raw(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Why a block's reader stopped reading a connection.
enum Read {
    /// The connection ended: wait for it to be revived.
    Lost,
    /// The block closed, the window dropped the pane or a checkpoint was
    /// bad: the pane ends.
    Ended,
}

impl Block {
    /// Connects to `block`, attaches and starts its reader.
    fn open(
        socket: &Path,
        block: BlockId,
        place: (u16, u16),
    ) -> Result<(PaneWriter, Inbox<Feed>), String> {
        let by = Instant::now() + HANDSHAKE;
        let opened = reach_with(socket, block, None, BLOCK, by, &NEVER).map_err(|m| match m {
            Missed::Retry(why) | Missed::Broken(why, _) => why,
            Missed::Dropped => "the server let the client go".into(),
        })?;
        let (revive, revived) = std::sync::mpsc::channel();
        let shared = Arc::new(Block {
            block,
            client: opened.client,
            stream: Mutex::new(opened.stream),
            conn: Mutex::new(opened.writer),
            takeover: Mutex::new(opened.takeover),
            revive: Mutex::new(Some(revive)),
            next: AtomicU64::new(1),
            lease: Mutex::new(Lease::at(place)),
            requests: opened.features.contains(Features::LEASE_REQUESTS),
            to_window: Mutex::new(None),
            told: Mutex::new(None),
            lists: opened
                .features
                .contains(Features::BLOCK_CALL.union(Features::DIR_LISTING)),
            listings: Mutex::new(HashMap::new()),
            clipboard: opened.features.contains(Features::CLIPBOARD),
            forwards: opened.features.contains(Features::PORT_FORWARD),
            forwarder: Mutex::new(None),
        });
        shared.send(|req| Message::Attach { req, block });
        let (tx, feed) = std::sync::mpsc::sync_channel(FEED_AHEAD);
        *lock(&shared.to_window) = Some(tx.clone());
        shared.tell(Take::Rest);
        let reader = Arc::clone(&shared);
        let conn = opened.conn;
        std::thread::Builder::new()
            .name(format!("mux-block-{block}"))
            .spawn(move || reader.read(conn, &revived, &tx))
            .map_err(|e| e.to_string())?;
        Ok((PaneWriter(shared), feed))
    }

    fn lease(&self) -> MutexGuard<'_, Lease> {
        self.lease.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How to take the block's client over now.
    fn takeover(&self) -> Option<Takeover> {
        *lock(&self.takeover)
    }

    /// Hands the block the connection `opened` took its client over on
    /// (handoff D110): the reader moves to it, where the server sends the
    /// screen again (an attached client's takeover), and the lease is
    /// asked for again if wanted, as silence may have lost it.
    fn revive(&self, opened: Opened) {
        *lock(&self.takeover) = opened.takeover;
        *lock(&self.conn) = opened.writer;
        let old = std::mem::replace(&mut *lock(&self.stream), opened.stream);
        // The reader leaves the old connection once it ends.
        let _ = old.shutdown(Shutdown::Both);
        if let Some(revive) = &*lock(&self.revive) {
            let _ = revive.send(opened.conn);
        }
        self.update(|lease, send| lease.revived(send));
    }

    fn send(&self, make: impl FnOnce(u64) -> Message) -> u64 {
        let req = self.next.fetch_add(1, Ordering::Relaxed);
        let conn = &mut self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        // A failed send ends the reader too, which ends the pane.
        let _ = conn.send(&make(req));
        req
    }

    /// Asks the server to list `path` (the block's folder when `None`) and
    /// calls `done` with the answer, or with why there is none.
    fn list_dir(&self, path: Option<String>, done: ListDone) {
        let block = self.block;
        let mut done = Some(done);
        self.send(|req| {
            if let Some(done) = done.take() {
                lock(&self.listings).insert(req, done);
            }
            Message::BlockCall {
                req,
                block,
                method: "terminal.list-dir".into(),
                args: path
                    .map(|p| ("path".into(), Value::Text(p)))
                    .into_iter()
                    .collect(),
            }
        });
    }

    /// Answers every waiting listing with `why`: its connection is gone.
    fn fail_listings(&self, why: &str) {
        let waiting: Vec<_> = lock(&self.listings).drain().collect();
        for (_, done) in waiting {
            done(Err(why.into()));
        }
    }

    /// Hands a listing's answer to whoever waits for `req`; whether
    /// someone did.
    fn listed(&self, req: u64, answer: Listed) -> bool {
        let done = lock(&self.listings).remove(&req);
        done.map(|done| done(answer)).is_some()
    }

    /// Listens on this machine for `port` of the block's host (C81) and
    /// calls `done` with the local port. Needs the lease, as the server
    /// does; a port already listened on answers with its listener. What
    /// the server later refuses closes the connection that asked.
    fn forward(self: &Arc<Self>, port: u16, done: ForwardDone) {
        if !self.lease().held {
            return done(Err(
                "Another client holds this block's input; take it over to forward.".into(),
            ));
        }
        let (answer, answered) = channel();
        let generation = {
            let mut forwarder = lock(&self.forwarder);
            if let Some(local) = forwarder.as_ref().and_then(|f| f.ports.get(&port)) {
                return done(Ok(*local));
            }
            let run = forwarder.get_or_insert_with(|| self.start_forwarder());
            let generation = Arc::clone(&run.stop);
            let want = Want {
                spec: Spec {
                    local: None,
                    remote: port,
                },
                answer,
            };
            match run.wants.try_send(want) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    return done(Err("too many forwards are waiting; try again".into()));
                }
                Err(TrySendError::Disconnected(_)) => {
                    *forwarder = None;
                    return done(Err("the forward ended; try again".into()));
                }
            }
            generation
        };
        let block = Arc::clone(self);
        std::thread::spawn(move || {
            let local = match answered.recv_timeout(LISTEN_WAIT) {
                Ok(Ok(bound)) => bound.local,
                Ok(Err(e)) => return done(Err(format!("could not listen here: {e}"))),
                Err(_) => return done(Err("listening here took too long".into())),
            };
            // The run that listened may be gone, and another its replacement:
            // only its own forward counts.
            match lock(&block.forwarder).as_mut() {
                Some(run) if Arc::ptr_eq(&run.stop, &generation) => {
                    run.ports.insert(port, local);
                }
                _ => return done(Err("the forward ended; try again".into())),
            }
            done(Ok(local));
        });
    }

    /// Starts the run that tunnels this block's ports, over its own
    /// connection: the reader passes it the frames it is for.
    fn start_forwarder(self: &Arc<Self>) -> Forwarder {
        let (frames_tx, frames) = sync_channel(FORWARD_FRAMES);
        let (wants_tx, wants) = sync_channel(FORWARD_WANTS);
        let stop = Arc::new(AtomicBool::new(false));
        let (block, flag) = (Arc::clone(self), Arc::clone(&stop));
        let spawned = std::thread::Builder::new()
            .name(format!("mux-forward-{}", self.block))
            .spawn(move || {
                let ended = forward::run_fed(
                    block.block,
                    FORWARD_REQ,
                    frames,
                    &wants,
                    ConnWriter(Arc::clone(&block), Arc::clone(&flag)),
                    &|| flag.load(Ordering::SeqCst),
                );
                if let forward::Ended::Lost(e) = ended {
                    eprintln!("rusty-mux-gui: block {}: forwards ended: {e}", block.block);
                }
                // Not a newer run of the same block.
                let mut current = lock(&block.forwarder);
                if current
                    .as_ref()
                    .is_some_and(|f| Arc::ptr_eq(&f.stop, &flag))
                {
                    *current = None;
                }
            });
        if let Err(e) = spawned {
            eprintln!(
                "rusty-mux-gui: block {}: starting forwards: {e}",
                self.block
            );
        }
        Forwarder {
            frames: frames_tx,
            wants: wants_tx,
            stop,
            ports: HashMap::new(),
        }
    }

    /// Ends the block's forwards, if any: its connection is going away.
    fn stop_forwards(&self) {
        if let Some(run) = lock(&self.forwarder).take() {
            run.stop.store(true, Ordering::SeqCst);
        }
    }

    /// Hands the forward run a frame it is for.
    fn to_forwards(&self, message: Message) {
        let mut forwarder = lock(&self.forwarder);
        let Some(run) = forwarder.as_ref() else {
            return;
        };
        // The reader never waits: a run that cannot keep up is ended, and
        // the next click starts a fresh one.
        if let Err(TrySendError::Full(_)) = run.frames.try_send(message) {
            eprintln!("rusty-mux-gui: block {}: forwards fell behind", self.block);
            if let Some(run) = forwarder.take() {
                run.stop.store(true, Ordering::SeqCst);
            }
        }
    }

    /// Asks for the lease when `wanted`, or gives it back when not.
    fn want_lease(&self, wanted: bool) {
        self.update(|lease, send| lease.want(wanted, send));
    }

    /// Runs `f` on the lease with a way to send its requests, then fits
    /// the block to its place and tells the window where taking it over
    /// stands.
    fn update(&self, f: impl FnOnce(&mut Lease, &mut dyn FnMut(LeaseAsk) -> u64)) {
        let block = self.block;
        let mut lease = self.lease();
        let mut send = |ask: LeaseAsk| {
            self.send(|req| match ask {
                LeaseAsk::Acquire => Message::AcquireLease { req, block },
                LeaseAsk::Release => Message::ReleaseLease { req, block },
                LeaseAsk::Request => Message::RequestLease { req, block },
            })
        };
        f(&mut lease, &mut send);
        self.fit(&mut lease);
        let take = lease.take();
        // Not under the lease: the feed may block while it is full.
        drop(lease);
        self.tell(take);
    }

    /// Tells the window `take`, when it changed and the server takes
    /// lease requests (gui-design D64).
    fn tell(&self, take: Take) {
        if !self.requests {
            return;
        }
        let mut told = lock(&self.told);
        if *told == Some(take) {
            return;
        }
        *told = Some(take);
        if let Some(tell) = &*lock(&self.to_window) {
            // A window gone ends the reader too.
            let _ = tell.send(Feed::Take(take));
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

    /// Reads the block's connections into feed steps, each revived one
    /// after the last ends, until the block closes or the window drops
    /// the pane.
    fn read(
        &self,
        mut conn: Conn<UnixStream>,
        revived: &Inbox<Conn<UnixStream>>,
        feed: &SyncSender<Feed>,
    ) {
        while let Read::Lost = self.read_one(&mut conn, feed) {
            self.fail_listings("the connection to the server was lost");
            // Its tunnels are gone with the connection.
            self.stop_forwards();
            match revived.recv() {
                Ok(next) => conn = next,
                Err(_) => break,
            }
        }
        // The feed ends with the reader, which ends the pane.
        *lock(&self.to_window) = None;
        self.fail_listings("the block ended");
        self.stop_forwards();
    }

    /// Reads one connection into feed steps; the screen comes first, as
    /// a checkpoint.
    fn read_one(&self, conn: &mut Conn<UnixStream>, feed: &SyncSender<Feed>) -> Read {
        let block = self.block;
        let mut checkpoint = Some(Receiver::default());
        // `None` once the history stream failed: the rest is ignored.
        let mut history = Some(HistoryReceiver::default());
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
                        return Read::Ended;
                    }
                    continue;
                }
                Message::Ready { block: b } if b == block => {
                    match checkpoint.take().map(Receiver::finish) {
                        Some(Ok((grid, parser))) => Feed::Screen(Box::new(grid), Box::new(parser)),
                        _ => {
                            eprintln!("rusty-mux-gui: block {block}: a bad checkpoint");
                            return Read::Ended;
                        }
                    }
                }
                Message::History {
                    block: b,
                    offset,
                    total,
                    data,
                } if b == block && checkpoint.is_none() => {
                    let Some(receiver) = history.as_mut() else {
                        continue;
                    };
                    match receiver.push(offset, total, &data) {
                        Ok(Some(batch)) => Feed::History(batch),
                        Ok(None) => continue,
                        Err(err) => {
                            // The screen stands; only older history is lost.
                            eprintln!("rusty-mux-gui: block {block}: bad history: {err}");
                            history = None;
                            continue;
                        }
                    }
                }
                Message::Output { block: b, data } if b == block => Feed::Bytes(data),
                Message::ClipboardRead {
                    block: b,
                    selection,
                } if b == block => Feed::ClipboardRead {
                    primary: selection == Selection::Primary,
                },
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
                } if b == block => return Read::Ended,
                Message::Event {
                    event: Event::LeaseChanged { block: b, holder },
                    ..
                } if b == block => {
                    // Asked for again once free while wanted (D115); never
                    // taken from a holder.
                    if holder != Some(self.client) {
                        self.update(|lease, send| lease.moved(holder.is_none(), send));
                    }
                    continue;
                }
                Message::Event {
                    event:
                        Event::GrantChanged {
                            block: b,
                            client,
                            until,
                        },
                    ..
                } if b == block && client == self.client => {
                    self.update(|lease, send| lease.granted(until.is_some(), send));
                    continue;
                }
                forward @ (Message::ForwardData { .. }
                | Message::ForwardAck { .. }
                | Message::CloseForward { .. }) => {
                    self.to_forwards(forward);
                    continue;
                }
                answer @ (Message::Done { req, .. } | Message::Failed { req, .. })
                    if req >= FORWARD_REQ =>
                {
                    self.to_forwards(answer);
                    continue;
                }
                Message::Text { req, text } => {
                    self.listed(req, Ok(text));
                    continue;
                }
                Message::Failed { req, reason, .. } if self.listed(req, Err(reason.clone())) => {
                    continue;
                }
                Message::Done { req, .. } => {
                    self.update(|lease, send| lease.answered(req, None, send));
                    continue;
                }
                Message::Failed { req, reason, .. } => {
                    self.update(|lease, send| lease.answered(req, Some(&reason), send));
                    continue;
                }
                _ => continue,
            };
            if feed.send(step).is_err() {
                return Read::Ended;
            }
        }
        Read::Lost
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

    /// Answers the block's lease holder's read (C54); the server refuses it
    /// from anyone else.
    fn clipboard_reply(&mut self, primary: bool, text: Option<String>) {
        let block = self.0.block;
        let selection = if primary {
            Selection::Primary
        } else {
            Selection::Clipboard
        };
        // Over the limit the server refuses the answer and the program would
        // wait; deny it instead.
        let data = text.map(String::into_bytes).filter(|d| {
            let fits = d.len() <= MAX_CLIPBOARD;
            if !fits {
                eprintln!(
                    "rusty-mux-gui: clipboard answer of {} bytes denied (over the limit)",
                    d.len()
                );
            }
            fits
        });
        self.0.send(|req| Message::ClipboardReply {
            req,
            block,
            selection,
            data,
        });
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
        // No revive: the reader ends with the connection.
        lock(&self.0.revive).take();
        self.0.send(|req| Message::Detach { req });
        let _ = lock(&self.0.stream).shutdown(Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use rusty_mux_core::{BlockId, InputMode, SessionId, Size, WindowId};
    use rusty_mux_proto::{Entry, Message};
    use rusty_uuid::Uuid;

    use super::{anchor, first_block, forwarded};

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_new_window_keeps_the_renderer_config_and_profile_but_not_the_target() {
        let given = args(&[
            "--gpu",
            "--config",
            "a.toml",
            "--profile=work",
            "--cwd",
            "--gpu",
            "/run/mux.sock",
            "--",
            "vim",
        ]);
        assert_eq!(
            forwarded(given.into_iter()),
            args(&["--gpu", "--config", "a.toml", "--profile=work"])
        );
    }

    fn entry(entry: Entry) -> Message {
        Message::Entry { req: 1, entry }
    }

    fn window(window: WindowId, session: SessionId) -> Message {
        laid(window, session, None)
    }

    fn laid(window: WindowId, session: SessionId, shown: Option<BlockId>) -> Message {
        entry(Entry::Window {
            window,
            session,
            name: String::new(),
            layout: shown.map(rusty_mux_core::Layout::Block),
            zoomed: None,
            focused: None,
        })
    }

    fn block(block: BlockId, window: WindowId) -> Message {
        entry(Entry::Block {
            block,
            window,
            size: Size::new(80, 24).expect("a size"),
            holder: None,
            title: String::new(),
            name: String::new(),
            owner: None,
            input: InputMode::Open,
            exited: false,
        })
    }

    #[test]
    fn a_new_window_opens_the_active_window_else_the_first_one_with_a_block() {
        let s = SessionId::from_uuid(Uuid::new_v4());
        let [w1, w2] = [(); 2].map(|_| WindowId::from_uuid(Uuid::new_v4()));
        let b = BlockId::from_uuid(Uuid::new_v4());
        let session = |active| {
            entry(Entry::Session {
                session: s,
                name: "s".into(),
                active,
                generated: false,
            })
        };
        // The first window is empty: the block in the second is found.
        let list = [
            session(None),
            window(w1, s),
            laid(w2, s, Some(b)),
            block(b, w2),
        ];
        assert_eq!(first_block(&list, s), Some(b));
        // With an active window, that one's block comes first.
        let other = BlockId::from_uuid(Uuid::new_v4());
        let list = [
            session(Some(w2)),
            laid(w1, s, Some(other)),
            laid(w2, s, Some(b)),
            block(other, w1),
            block(b, w2),
        ];
        assert_eq!(first_block(&list, s), Some(b));
        // A detached block is skipped for a laid-out one in another window.
        let list = [
            session(Some(w1)),
            laid(w1, s, None),
            block(other, w1),
            laid(w2, s, Some(b)),
            block(b, w2),
        ];
        assert_eq!(first_block(&list, s), Some(b));
        // Floats are no block to show: a tile beside them is, floats alone are not.
        let float = |window, block| {
            entry(Entry::Floats {
                window,
                floats: vec![rusty_mux_core::Float {
                    block,
                    at: rusty_mux_core::FloatRect::new(10, 10, 40, 40).expect("a float"),
                }],
            })
        };
        let list = [
            session(None),
            laid(w1, s, Some(b)),
            float(w1, other),
            block(b, w1),
            block(other, w1),
        ];
        assert_eq!(first_block(&list, s), Some(b), "mixed");
        let list = [
            session(None),
            window(w1, s),
            float(w1, other),
            block(other, w1),
        ];
        assert_eq!(first_block(&list, s), None, "floats only");
        // No block anywhere.
        assert_eq!(first_block(&[session(None), window(w1, s)], s), None);
    }

    fn blocks() -> [BlockId; 3] {
        std::array::from_fn(|_| BlockId::from_uuid(Uuid::new_v4()))
    }

    #[test]
    fn a_reattached_block_goes_beside_the_focused_tile() {
        let b = blocks();
        assert_eq!(anchor(&[b[0], b[1]], Some(b[1])), Some(b[1]));
    }

    #[test]
    fn a_focused_float_is_no_anchor_the_first_tile_is() {
        let b = blocks();
        assert_eq!(anchor(&[b[0], b[1]], Some(b[2])), Some(b[0]));
        assert_eq!(anchor(&[b[0]], None), Some(b[0]), "no focus at all");
    }

    #[test]
    fn a_window_with_no_tile_has_no_anchor() {
        let b = blocks();
        assert_eq!(anchor(&[], Some(b[0])), None, "floats alone");
        assert_eq!(anchor(&[], None), None);
    }
}

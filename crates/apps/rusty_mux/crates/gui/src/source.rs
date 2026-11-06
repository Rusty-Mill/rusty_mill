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
use rusty_mux_core::{BlockId, ClientId, Event, Layout, SessionId, Side, Size, WindowId};
use rusty_mux_proto::{Features, Launch, Message};
use rusty_term::backend::BackendHandle;
use rusty_term::gui::source::{
    Act, BlockInfo, Detached, Dir, Feed, PaneIo, Remote, Session, Source, Tree,
};

use crate::headers::{Call, Effect, Headers};
use crate::keys::Keys;
use crate::session::{Change, Model, StripKeys, local_host};
use crate::tree::tree;

/// What the GUI's control connection asks for: the TUI's set, plus names
/// for the strip and who each client is, to count them, and block calls
/// and the events that say a block's folder, status or foreground program
/// changed, for its header (handoff D105). A server without some grants
/// the rest, and the chrome shows less.
pub const GUI: Features = CONTROL
    .union(Features::LABELS)
    .union(Features::IDENTITY)
    .union(Features::BLOCK_CALL)
    .union(Features::STATUS)
    .union(Features::TERMINAL_EVENTS)
    .union(Features::PROCESS)
    .union(Features::MORE_PROGRAM_EVENTS);

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
        features: GUI,
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
    let entries = req.call(|req| Message::List { req })?;
    let follows_active = welcomed.features.contains(Features::ACTIVE_WINDOW);
    let model = Model::from_list(&entries, block, local_host(), follows_active)?;
    let calls_blocks = welcomed.features.contains(Features::BLOCK_CALL);
    let next = req.next;
    let layout = model.view().0.cloned().unwrap_or(Layout::Block(block));
    let state = Arc::new(State {
        socket: socket.to_owned(),
        control: Mutex::new(Conn::new(clone(&stream)?)),
        next: AtomicU64::new(next),
        tells_focus: welcomed.features.contains(Features::FOCUS),
        edits_layout: welcomed.features.contains(Features::LAYOUT_EDIT),
        selects: follows_active,
        calls_blocks,
        session: Mutex::new(model.session_id()),
        switch: Mutex::new(None),
        shown: Mutex::new(Shown::default()),
    });
    let (first, focus) = {
        let mut shown = state.shown();
        let first = tree(&layout, &mut shown.keys);
        let focus = shown.keys.key(block);
        shown.focus = Some(focus);
        shown.window = Some(model.shown());
        (first, focus)
    };
    state.tell_focus(block);
    let mut events = Events {
        state: Arc::clone(&state),
        conn,
        model,
        headers: Headers::default(),
    };
    events.follow_blocks();
    for m in &entries {
        if let Message::Entry { entry, .. } = m {
            events.headers.take(entry);
        }
    }
    Ok(Attached {
        block,
        focus,
        source: MuxSource(state),
        first,
        events,
    })
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
    /// The server answers `BlockCall`, which the headers read with.
    calls_blocks: bool,
    /// The session shown, where a new window goes.
    session: Mutex<SessionId>,
    /// The session the user picked, for the events reader to show.
    switch: Mutex<Option<SessionId>>,
    shown: Mutex<Shown>,
}

/// The blocks shown and which has the focus.
#[derive(Default)]
struct Shown {
    keys: Keys,
    /// The strip's tab keys for the session's windows.
    windows: Keys<WindowId>,
    /// The switcher's keys for the server's sessions.
    sessions: Keys<SessionId>,
    /// Each open pane's block connection, kept weakly: the window owns it.
    blocks: HashMap<u64, std::sync::Weak<Block>>,
    focus: Option<u64>,
    /// The server window shown.
    window: Option<WindowId>,
}

impl State {
    fn shown(&self) -> MutexGuard<'_, Shown> {
        lock(&self.shown)
    }

    /// Tells the server the window's focus moved to `block`.
    fn tell_focus(&self, block: BlockId) {
        if self.tells_focus {
            self.send(|req| Message::FocusBlock { req, block });
        }
    }

    /// The clients this window's block connections are.
    fn mine(&self) -> Vec<ClientId> {
        let shown = self.shown();
        shown
            .blocks
            .values()
            .filter_map(std::sync::Weak::upgrade)
            .map(|b| b.client)
            .collect()
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
        Ok(PaneIo {
            writer: Box::new(pane),
            feed: Box::new(feed.into_iter()),
            // The server's own terminal answers the program's queries.
            answers_queries: false,
        })
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
        let (block, window, focused) = {
            let shown = self.0.shown();
            let key = match act {
                Act::Split { key, .. } | Act::Zoom(key) | Act::Close(key) | Act::Reattach(key) => {
                    Some(key)
                }
                Act::Unzoom => None,
            };
            let focused = shown.focus.and_then(|k| shown.keys.id(k));
            (key.and_then(|k| shown.keys.id(k)), shown.window, focused)
        };
        self.0.try_send(|req| match act {
            Act::Split { dir, .. } => block.map(|block| Message::Split {
                req,
                block,
                side: match dir {
                    Dir::Vertical => Side::Right,
                    Dir::Horizontal => Side::Down,
                },
                launch: Launch::default(),
            }),
            Act::Zoom(_) => block.map(|block| Message::Zoom { req, block }),
            Act::Close(_) => block.map(|block| Message::CloseBlock { req, block }),
            Act::Unzoom => window.map(|window| Message::Unzoom { req, window }),
            // Beside the focused block; the server fills an empty window.
            Act::Reattach(_) => block.map(|block| Message::MoveBlock {
                req,
                block,
                beside: focused,
                side: Side::Right,
            }),
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

    /// Creates a window with one block and selects it, on a connection of
    /// its own so the UI never waits for the server.
    fn new_window(&self) {
        let (socket, session) = (self.0.socket.clone(), *lock(&self.0.session));
        let spawned = std::thread::Builder::new()
            .name("mux-new-window".into())
            .spawn(move || {
                if let Err(e) = create_window(&socket, session) {
                    eprintln!("rusty-mux-gui: new window: {e}");
                }
            });
        if let Err(e) = spawned {
            eprintln!("rusty-mux-gui: new window: {e}");
        }
    }
}

/// Creates a window in `session` with one block, and makes it active.
/// The connection asks for no takeover, so the server forgets it at once;
/// the handshake needs checkpoints.
fn create_window(socket: &Path, session: SessionId) -> Result<(), String> {
    let stream = connect(socket)?;
    let mut conn = Conn::new(stream);
    conn.send(&Message::Hello {
        features: Features::CHECKPOINT_V6.union(Features::ACTIVE_WINDOW),
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
        launch: Launch::default(),
    })?;
    req.call(|req| Message::SelectWindow { req, window })?;
    Ok(())
}

/// What the control connection tells the window.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// Show this layout; `None` closes the window, and comes last.
    Layout(Option<Tree>),
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
}

/// The control connection's events for the window.
pub struct Events {
    state: Arc<State>,
    conn: Conn<UnixStream>,
    model: Model,
    headers: Headers,
}

impl Events {
    /// Follows the session on a thread of its own, telling `remote`; the
    /// session's end, or the connection's, closes the window.
    pub fn follow(self, remote: Remote) {
        std::thread::spawn(move || {
            self.run(|out| match out {
                Out::Layout(t) => remote.layout(t),
                Out::Focus(k) => remote.focus(k),
                Out::Zoom(k) => remote.zoom(k),
                Out::Empty => remote.empty(),
                Out::Session(s) => remote.session(s),
                Out::Blocks(b) => remote.blocks(b),
            })
        });
    }

    /// Follows the session until it or the connection ends, or `out` says
    /// `false`. The session goes out first, then every change.
    pub fn run(mut self, mut out: impl FnMut(Out) -> bool) {
        if !(out(Out::Session(self.session())) && out(Out::Zoom(self.zoomed()))) {
            return;
        }
        while let Ok(message) = self.conn.recv() {
            if !self.headed(&message, &mut out) {
                return;
            }
            let change = match &message {
                Message::Event { event, .. } => self.model.apply(event),
                Message::Entry { entry, .. } => self.model.take(entry),
                _ => Change::None,
            };
            if !self.switched(&mut out) {
                return;
            }
            if let Message::Event {
                event: Event::WindowCreated { .. } | Event::SessionCreated { .. },
                ..
            } = message
            {
                // Its name comes with its entry.
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
                // The strip follows: the thumbnail and detached blocks.
                Change::Layout => self.layout(&mut out) && out(Out::Session(self.session())),
                Change::Focus(block) => {
                    let key = self.state.shown().keys.key(block);
                    self.state.focus(key);
                    out(Out::Focus(key))
                }
                Change::Shown => {
                    self.layout(&mut out)
                        && self.focus_shown(&mut out)
                        && out(Out::Session(self.session()))
                }
                Change::Emptied(window) => {
                    self.state.send(|req| Message::SelectWindow { req, window });
                    out(Out::Session(self.session()))
                }
                Change::Gone => break,
            };
            if !go_on {
                return;
            }
        }
        out(Out::Layout(None));
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
        !changed || out(Out::Blocks(self.blocks()))
    }

    /// What the shown blocks' headers say.
    fn blocks(&self) -> Vec<(u64, BlockInfo)> {
        let mine = self.state.mine();
        let keys = &mut self.state.shown().keys;
        self.headers
            .infos(keys, &mine, |c| self.model.client_name(c))
    }

    /// The session as the strip shows it.
    fn session(&self) -> Session {
        let shown = &mut *self.state.shown();
        let mut session = self.model.session(&mut StripKeys {
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
        let shown = layout.map(|l| tree(l, &mut self.state.shown().keys));
        self.keep_focus_in(layout.map(Layout::blocks).unwrap_or_default(), focused);
        if shown.is_none() && !self.model.detached().is_empty() {
            return out(Out::Empty);
        }
        let ended = shown.is_none();
        out(Out::Layout(shown)) && !ended && out(Out::Zoom(self.zoomed()))
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
struct Block {
    block: BlockId,
    /// The client the server made of this connection.
    client: ClientId,
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
            client: opened.client,
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

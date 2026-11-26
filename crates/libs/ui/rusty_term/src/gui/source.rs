//! Where a pane's terminal comes from: the window's session-source port
//! (handoff pack D101).
//!
//! The window draws a pane from a [`PaneIo`]: a [`Feed`] of what to show
//! and a writer for input and size requests: a rusty_mux block, attached
//! from `rusty_mux_gui` (local mode retired, D159). A window
//! ([`super::run_attached`]) takes its panes and their layout from a
//! [`Source`].

use std::io;
use std::time::Instant;

use winit::event_loop::EventLoopProxy;

use super::window::UserEvent;
use crate::backend::BackendHandle;
use crate::core::{AnsiParser, Grid};

pub use super::layout::Dir;

/// One step of what a pane shows.
pub enum Feed {
    /// Terminal output to parse into the pane's grid.
    Bytes(Vec<u8>),
    /// A whole terminal to show instead of the pane's own, such as a mux
    /// block's screen on attach.
    Screen(Box<Grid>, Box<AnsiParser>),
    /// A batch of older history for the screen last shown (mux C03),
    /// newest batch first: it goes above the history the pane holds.
    History(Vec<u8>),
    /// The source resized the pane: a mux block's size is the server's.
    Resized { cols: u16, rows: u16 },
    /// Where this window's taking over of an owner-only block stands
    /// (gui-design D64, handoff D153); first sent once the source can ask.
    Take(Take),
    /// The pane's program reads a selection (OSC 52 `?`) and this window
    /// holds the block's input lease, so it is asked (C54, handoff D309,
    /// D310). The window answers by its `clipboard` policy through
    /// [`BackendHandle::clipboard_reply`]; only a pane whose
    /// [`PaneIo::serves_clipboard`] is set gets these.
    ClipboardRead { primary: bool },
    /// The server ended a forward of the pane with a reason, such as a dial
    /// nothing answered (C81): shown as a notice.
    ForwardRefused { port: u16, reason: String },
}

/// Where this window stands on typing into an owner-only block, whose
/// owner grants who may (gui-design D64, handoff D153).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Take {
    /// Not taking it over: "Take over".
    #[default]
    Rest,
    /// Asked the owner, waiting for a grant: "Cancel ask".
    Asking,
    /// Granted and holding the lease: "You have input", "Give back".
    Held,
    /// The ask was refused.
    Refused(Refusal),
    /// The grant ended, by a revoke or its time running out.
    Ended,
}

/// Why an ask to take a block over was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Its owner is reconnecting or can't hear asks.
    Unreachable,
    /// Anything else.
    Other,
}

/// A pane's two ends. Dropping it ends the pane: its feed stops and its
/// writer hangs up (a local shell's PTY, a mux block's connection).
pub struct PaneIo {
    /// Takes the pane's input and size requests.
    pub writer: Box<dyn BackendHandle>,
    /// What to show, blocking until the next step; ends with the pane.
    pub feed: Box<dyn Iterator<Item = Feed> + Send>,
    /// Whether the window answers the program's queries (DA, DSR). A mux
    /// server's own terminal already does, so its blocks say `false`.
    pub answers_queries: bool,
    /// The source delivers the program's clipboard reads as
    /// [`Feed::ClipboardRead`] and takes the answer, so the window does not
    /// answer one from the pane's own grid as well: it would be answered
    /// twice, once unasked (C54, handoff D312).
    pub serves_clipboard: bool,
}

/// A window's panes as a source lays them out: leaves are the source's pane
/// keys.
#[derive(Clone, Debug, PartialEq)]
pub enum Tree {
    Pane(u64),
    /// `ratio` of the area goes to `a`, the left or top child.
    Split {
        dir: Dir,
        ratio: f32,
        a: Box<Tree>,
        b: Box<Tree>,
    },
}

/// Where a float sits, in whole percent of the body from its top left
/// (mux C42; gui-design D89). A source may refuse a place it can't keep;
/// the window never asks for one under 10% wide or tall or outside the
/// body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloatAt {
    pub x: u8,
    pub y: u8,
    pub w: u8,
    pub h: u8,
}

/// A pane that floats over the tiled layout, and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Float {
    /// Its pane key.
    pub key: u64,
    pub at: FloatAt,
}

/// What an attached window's top strip shows of its source's session
/// (gui-design F1, D46).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub name: String,
    /// Where the session runs, as this client reached it: "on {host}".
    pub host: String,
    /// The session's windows, in order; one is shown.
    pub windows: Vec<WindowTab>,
    /// Clients attached to the session, this one included.
    pub clients: usize,
    /// The session runs on this machine, so its paths can be read here
    /// (a block header's git branch, gui-design D48).
    pub local: bool,
    /// Every session on the server, this one included, in order, for the
    /// switcher (gui-design F4, D60).
    pub sessions: Vec<SessionChoice>,
    /// The shown window's detached blocks (gui-design F6, D61), newest
    /// first; empty when the source can't put one back.
    pub detached: Vec<Detached>,
    /// The account's other windows that can be asked to show a session
    /// (C64, gui-design D94), in the order they are numbered; empty when
    /// the source can't ask.
    pub peers: Vec<Peer>,
    /// The session's blocks by the card their agent works (D336, gui-design
    /// D97); `None` when the source can't tell who works a block, which
    /// hides the session overview.
    pub overview: Option<Vec<OverviewCard>>,
}

/// One card of the session overview: the blocks worked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverviewCard {
    /// The card's id; `None` for the blocks with no card.
    pub card: Option<String>,
    pub tiles: Vec<OverviewTile>,
}

/// A block as the session overview's tile shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverviewTile {
    /// Its pane key.
    pub key: u64,
    /// The source's key for the window it is in.
    pub window: u64,
    /// The agent's name, else the block's name or title; empty for neither.
    pub name: String,
    /// The agent's role; empty for a block no agent works.
    pub role: String,
    /// The program has ended and the block is held.
    pub exited: bool,
    /// The block is out of its window's layout.
    pub detached: bool,
    /// The client holding its input, when that isn't this window.
    pub holder: Option<String>,
}

/// Another window of the account (C64).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// The source's key for the window.
    pub key: u64,
    /// "Window {n}", with the client's name when it has one (gui-design
    /// D94).
    pub label: String,
}

/// Whom to ask to show a session (C64).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    /// One other window, by [`Peer::key`].
    Window(u64),
    /// Every other window of the account.
    All,
}

/// A block taken out of the shown window's layout, still running.
#[derive(Clone, Debug, PartialEq)]
pub struct Detached {
    /// Its pane key.
    pub key: u64,
    /// Its name, else its title, else its program.
    pub name: String,
    /// When this client saw it go; `None` when it went before.
    pub since: Option<Instant>,
}

/// One of the server's sessions, as the switcher lists it (gui-design D60).
#[derive(Clone, Debug, PartialEq)]
pub struct SessionChoice {
    /// The source's key for the session.
    pub key: u64,
    pub name: String,
    /// Its windows.
    pub windows: usize,
    /// Distinct people attached to its blocks.
    pub clients: usize,
    /// Whether this is the session shown.
    pub current: bool,
    /// Whether it has blocks to show; a session without can't be picked.
    pub can_show: bool,
}

/// One of a session's windows, as its strip tab shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowTab {
    /// The source's key for the window.
    pub key: u64,
    pub name: String,
    /// Its blocks' layout, for the tab's thumbnail; `None` while unknown.
    pub tree: Option<Tree>,
    /// Whether this is the window shown.
    pub shown: bool,
}

/// What a block's header shows of it (gui-design D47, D48), as its source
/// tells it. The window decides how; a field it lacks is left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockInfo {
    /// The program's working directory.
    pub cwd: Option<String>,
    /// The program the block started, its shell.
    pub shell: Option<String>,
    /// The program in the foreground, when it isn't the shell.
    pub foreground: Option<String>,
    /// What the program reports doing (OSC 7501's top record).
    pub status: Option<Status>,
    /// Who else has input: the client holding the block's input lease,
    /// when that isn't this window.
    pub holder: Option<String>,
    /// Whose grant typing needs, for a block whose input is its owner's
    /// (C77); `None` for an open block.
    pub gate: Option<Gate>,
}

/// An owner-only block's owner (gui-design D64).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Its owner's name, which decides who types.
    Owned(String),
    /// Its owner is gone for good: nobody may type.
    Orphaned,
}

/// A program's status report (OSC 7501).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    /// What it says it does, such as "Editing zoom.rs".
    pub msg: Option<String>,
}

/// What a program says it is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Working,
    /// Waiting on the user, and why, when it says.
    Blocked(Option<Wait>),
    Done,
    Error,
}

/// Why a blocked program waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    Permission,
    Question,
    SignIn,
}

/// Where an attached window's panes come from, such as a rusty_mux window.
/// The source owns the layout: the window shows the [`Tree`]s it is sent
/// through [`Remote`] and never edits one itself (pack D28, D102).
pub trait Source {
    /// Opens pane `key`, placed `cols × rows`.
    fn open(&self, key: u64, cols: u16, rows: u16) -> io::Result<PaneIo>;
    /// The user moved the focus to pane `key`.
    fn focus(&self, key: u64);
    /// The user picked window `key` on the strip: show it instead.
    fn select(&self, key: u64);
    /// The user picked session `key` in the switcher: show it instead
    /// (gui-design D60). The source answers with its layout and session.
    fn switch(&self, key: u64);
    /// The user asked for a new window in the session.
    fn new_window(&self);
    /// The user asked other windows to show session `key` (C64, gui-design
    /// D93); `false` for a source that can't.
    fn show_in(&self, _key: u64, _to: Show) -> bool {
        false
    }
    /// The user asked for session `key` in a new window of its own (C64,
    /// gui-design D93); `false` for a source that can't.
    fn open_window(&self, _key: u64) -> bool {
        false
    }
    /// The user asked to change the layout (gui-design F2, F7; handoff
    /// D104). The source sends it on; the window shows the result when the
    /// source lays it out.
    fn act(&self, act: Act);
    /// The user asked to try reaching the server again now ("Retry now",
    /// "Retry"; gui-design D62). The source answers with [`Remote::link`].
    fn retry(&self);
    /// The user asked to stop trying ("Stop trying"); the source says
    /// [`Link::Down`].
    fn stop(&self);
    /// Whether the source floats panes (mux C42): the window then draws
    /// the floats it is told and offers the float keys and mouse. A source
    /// that doesn't is never told of any.
    fn floating(&self) -> bool {
        false
    }
    /// Whether the source can list a folder on pane `key`'s host for the
    /// command palette's directory browse (gui-design D78). A source that
    /// can't hides the rows.
    fn lists(&self, _key: u64) -> bool {
        false
    }
    /// Lists `path` (the pane's folder when `None`) on pane `key`'s host,
    /// and calls `done` once, from any thread, with the listing text
    /// ([`super::palette::Listing::parse`]) or why it failed. Not called
    /// when [`Source::lists`] is `false`.
    fn list_dir(&self, _key: u64, _path: Option<String>, done: ListDone) {
        done(Err("this source lists no folders".into()));
    }
    /// What the source can show of pane `key`'s workspace tree in the
    /// explorer panel (gui-design D99, card #1573). A source that can't
    /// hides the panel's entry points.
    fn explores(&self, _key: u64) -> Explore {
        Explore::No
    }
    /// The user opened, used or closed the explorer on pane `key`. The
    /// source owns the tree and answers with [`Remote::explorer`]. Not
    /// called when [`Source::explores`] is [`Explore::No`].
    fn explore(&self, _key: u64, _ask: Explored) {}
    /// Whether the source can tunnel a loopback port of pane `key`'s host
    /// to this machine (mux C81), so a `localhost` link in that pane opens
    /// here. A source that can't leaves such links to the ordinary link
    /// policy.
    fn forwards(&self, _key: u64) -> bool {
        false
    }
    /// Listens on this machine for connections to `port` of pane `key`'s
    /// host and calls `done` once, from any thread, with the local port or
    /// why not. A port already forwarded answers with its listener. Not
    /// called when [`Source::forwards`] is `false`.
    fn forward(&self, _key: u64, _port: u16, done: ForwardDone) {
        done(Err("this source forwards no ports".into()));
    }
    /// The local port pane `key` forwards `port` on right now, if its
    /// forward is still the one running: how a late answer is told from a
    /// live one.
    fn forwarded_port(&self, _key: u64, _port: u16) -> Option<Bound> {
        None
    }
    /// A stable, opaque name for pane `key`'s host, one line, the same on
    /// every connection to it (such as the server's socket): what a saved
    /// forward trust is kept under. `None` keeps trust for the window only.
    fn host_id(&self, _key: u64) -> Option<String> {
        None
    }
    /// Whether the source can send a file to pane `key`'s host (mux C82),
    /// so a file dropped or pasted on that pane is uploaded and its path
    /// there typed. A source that can't leaves the file's own path to be
    /// typed, as a local pane's host is this machine.
    fn uploads(&self, _key: u64) -> bool {
        false
    }
    /// Sends the file at `path` to pane `key`'s host and calls `done` once,
    /// from any thread, with its path there or why not. Not called when
    /// [`Source::uploads`] is `false`.
    fn upload(&self, _key: u64, _path: std::path::PathBuf, done: UploadDone) {
        done(Err("this source uploads no files".into()));
    }
}

/// How much of a block's workspace the source can show (gui-design D99).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Explore {
    /// Nothing: the server has no workspace call, or the block is not open.
    #[default]
    No,
    /// The tree and copy path.
    Tree,
    /// The tree, and the change marks and size/age footer too.
    Marked,
}

/// What the panel asks of the source's tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Explored {
    /// The panel opened: read the workspace root.
    Open,
    /// The panel closed: drop the tree.
    Close,
    /// A key or click in the panel.
    Do(super::explorer::Intent),
}

/// A forward that listens: the local port, and which run of the pane's
/// forwards holds it, so an answer from a run that has since been replaced
/// is never taken for the new one's, whatever port it listened on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bound {
    pub local: u16,
    pub run: u64,
}

/// The forward that listens, or why there is none.
pub type Forwarded = Result<Bound, String>;

/// What a source calls once with a forward's local port.
pub type ForwardDone = Box<dyn FnOnce(Forwarded) + Send>;

/// A file's path on the pane's host, or why it was not sent.
pub type Uploaded = Result<String, String>;

/// What a source calls once with an upload's outcome.
pub type UploadDone = Box<dyn FnOnce(Uploaded) + Send>;

/// A folder listing's text, or why the host refused it.
pub type Listed = Result<String, String>;

/// What a source calls once with a folder listing.
pub type ListDone = Box<dyn FnOnce(Listed) + Send>;

/// How an attached window's source reaches its server (gui-design D62,
/// handoff D110).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Link {
    /// Connected.
    #[default]
    Up,
    /// The connection was lost: trying again for `left` more seconds of
    /// the server's grace, an attempt running while `trying`.
    Reconnecting { left: u32, trying: bool },
    /// Back. `kept`: this client took its clients over, leases as they
    /// were (handoff D83). Otherwise it attached anew, its leases from
    /// before released, and the window opens its panes again.
    Back { kept: bool },
    /// Not trying: the grace ran out or the user stopped. A Retry runs
    /// while `trying`.
    Down { trying: bool },
}

/// A layout change the user asked an attached window's source for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Start a block beside pane `key`; `Dir::Vertical` puts it on the
    /// right, `Dir::Horizontal` below.
    Split { key: u64, dir: Dir },
    /// Show pane `key` alone.
    Zoom(u64),
    /// Show every pane again.
    Unzoom,
    /// Close pane `key`'s block.
    Close(u64),
    /// Put detached block `key` back, beside the focused pane (gui-design
    /// D61); the source places it.
    Reattach(u64),
    /// The user confirmed taking owner-only pane `key` over: ask its owner,
    /// and type once granted (gui-design D64, handoff D153).
    TakeOver(u64),
    /// Stop waiting for pane `key`'s owner; a later grant is not used.
    CancelAsk(u64),
    /// Give pane `key`'s input back; the grant stays the owner's to end.
    GiveBack(u64),
    /// Float pane `key` over the layout, where the source first puts a
    /// float (mux C42, gui-design D91).
    Float(u64),
    /// Move and resize float `key` to `at`, sent when a drag ends or a
    /// key steps it. The window shows the place the source tells next, so
    /// one it refuses snaps back.
    Place { key: u64, at: FloatAt },
    /// Raise float `key` over the others (`top`) or lower it under them.
    Stack { key: u64, top: bool },
}

/// The source's way into a running window, from any thread. Each call is
/// `false` once the window is gone.
#[derive(Clone)]
pub struct Remote(pub(crate) EventLoopProxy<UserEvent>);

/// An info notice for an attached window's banner (gui-design D26, D63),
/// such as a server older than its client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub detail: String,
}

impl Remote {
    /// Show `tree` with `floats` over it, back to front (mux C42); `None`
    /// closes the window (the source's window is gone). A float is a pane
    /// `tree` doesn't hold: told with the layout, a block that goes from
    /// one to the other keeps its pane.
    pub fn layout(&self, tree: Option<Tree>, floats: Vec<Float>) -> bool {
        self.0.send_event(UserEvent::Layout(tree, floats)).is_ok()
    }

    /// Show these floats, back to front, over the layout (mux C42): every
    /// float of the window, each time, when only they changed. A float is a
    /// pane the layout doesn't hold: its pane opens here, and closes when a
    /// later [`Remote::layout`] or this leaves it out.
    pub fn floats(&self, floats: Vec<Float>) -> bool {
        self.0.send_event(UserEvent::Floats(floats)).is_ok()
    }

    /// Move the focus to pane `key`, as another client did.
    pub fn focus(&self, key: u64) -> bool {
        self.0.send_event(UserEvent::Focus(key)).is_ok()
    }

    /// Show no pane: the window's blocks are all detached (gui-design
    /// D61). A later [`Remote::layout`] shows panes again.
    pub fn empty(&self) -> bool {
        self.0.send_event(UserEvent::Empty).is_ok()
    }

    /// Show pane `key` alone, or every pane with `None` (gui-design F7).
    /// Comes after the [`Remote::layout`] it applies to.
    pub fn zoom(&self, key: Option<u64>) -> bool {
        self.0.send_event(UserEvent::Zoom(key)).is_ok()
    }

    /// Show how the source reaches its server now (gui-design D62). A
    /// [`Link::Back`] with `kept: false` comes before the layout of the
    /// panes to open again.
    pub fn link(&self, link: Link) -> bool {
        self.0.send_event(UserEvent::Link(link)).is_ok()
    }

    /// Show that the shown session, `name`, ended while the source was
    /// away from its server: no block, and a way to pick another session
    /// (gui-design D62, handoff D114). A layout ends it.
    pub fn ended(&self, name: String) -> bool {
        self.0.send_event(UserEvent::Ended(name)).is_ok()
    }

    /// Show `notice` in the banner while the link is up, until the user
    /// dismisses it or a later notice replaces it; `None` takes it away
    /// (gui-design D63).
    pub fn notice(&self, notice: Option<Notice>) -> bool {
        self.0.send_event(UserEvent::Notice(notice)).is_ok()
    }

    /// Another window, `by`, asks this one to show session `key` (C64,
    /// gui-design D95, D96): the window switches and says so, or waits
    /// while a confirm, the palette or a link prompt is open.
    pub fn shown(&self, key: u64, by: String) -> bool {
        self.0.send_event(UserEvent::Shown(key, by)).is_ok()
    }

    /// Show `session` on the top strip.
    pub fn session(&self, session: Session) -> bool {
        self.0.send_event(UserEvent::Session(session)).is_ok()
    }

    /// What the session's blocks say, by pane key, for the shown panes'
    /// headers and the strip's thumbnails; a pane left out shows an empty
    /// header.
    pub fn blocks(&self, blocks: Vec<(u64, BlockInfo)>) -> bool {
        self.0.send_event(UserEvent::Blocks(blocks)).is_ok()
    }

    /// What the explorer panel shows now for pane `key`'s tree (gui-design
    /// D99); one for a pane the panel is not open on is dropped.
    pub fn explorer(&self, key: u64, snapshot: super::explorer::Snapshot) -> bool {
        self.0
            .send_event(UserEvent::Explorer(key, snapshot))
            .is_ok()
    }
}

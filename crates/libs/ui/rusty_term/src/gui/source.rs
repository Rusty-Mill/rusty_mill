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
    /// The source resized the pane: a mux block's size is the server's.
    Resized { cols: u16, rows: u16 },
    /// Where this window's taking over of an owner-only block stands
    /// (gui-design D64, handoff D153); first sent once the source can ask.
    Take(Take),
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
}

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
    /// Show `tree`; `None` closes the window (the source's window is gone).
    pub fn layout(&self, tree: Option<Tree>) -> bool {
        self.0.send_event(UserEvent::Layout(tree)).is_ok()
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
}

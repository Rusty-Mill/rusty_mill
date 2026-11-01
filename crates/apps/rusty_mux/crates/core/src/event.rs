//! Changes to the model, in the order they happened.

use crate::{BlockId, ClientId, Layout, Principal, SessionId, Size, Transport, WindowId};

/// One change made by [`Mux`](crate::Mux).
///
/// Closing a parent emits its children's events first, so a consumer that
/// applies events in order never sees a child outlive its parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    SessionCreated {
        session: SessionId,
    },
    SessionClosed {
        session: SessionId,
    },
    WindowCreated {
        window: WindowId,
        session: SessionId,
    },
    WindowClosed {
        window: WindowId,
    },
    BlockCreated {
        block: BlockId,
        window: WindowId,
        size: Size,
    },
    BlockResized {
        block: BlockId,
        size: Size,
    },
    BlockClosed {
        block: BlockId,
    },
    /// A new client, with the principal the server verified; `None` only
    /// when decoded from a server older than `Features::IDENTITY`.
    ClientConnected {
        client: ClientId,
        principal: Option<Principal>,
    },
    ClientDisconnected {
        client: ClientId,
    },
    /// A new connection took the client over, by `transport`: it keeps its
    /// id, name, attachment and lease.
    ClientConnectionChanged {
        client: ClientId,
        principal: Principal,
        transport: Transport,
    },
    Attached {
        client: ClientId,
        block: BlockId,
    },
    Detached {
        client: ClientId,
        block: BlockId,
    },
    /// The block's input lease moved; `None` means nobody may write.
    LeaseChanged {
        block: BlockId,
        holder: Option<ClientId>,
    },
    /// What the block's program says it is doing changed state; the
    /// `terminal.status` block method reads it.
    BlockStatus {
        block: BlockId,
    },
    /// The block's program reported a new working directory (OSC 7); the
    /// `terminal.pwd` block method reads it.
    PwdChanged {
        block: BlockId,
    },
    /// The block's title changed; the `terminal.title` block method reads it.
    TitleChanged {
        block: BlockId,
    },
    /// The block's program exited by itself; [`Event::BlockClosed`] follows.
    ProgramExited {
        block: BlockId,
        status: ExitStatus,
    },
    /// The block's program rang the bell. A burst read at once is one event.
    Bell {
        block: BlockId,
    },
    /// The progress the block's program shows changed (OSC 9;4).
    ProgressChanged {
        block: BlockId,
        progress: Progress,
    },
    /// The block's program asked for a desktop notification (OSC 9 or 777);
    /// an empty title leaves it to the client.
    Notification {
        block: BlockId,
        title: String,
        body: String,
    },
    /// Another process group took the block's terminal: a job started in
    /// the foreground, or the shell got it back. `pid` is the group's
    /// leader and `name` its program, empty when unknown; the
    /// `terminal.process` block method reads the rest.
    ProcessChanged {
        block: BlockId,
        pid: u32,
        name: String,
    },
    /// The session's name changed to `name`.
    SessionRenamed {
        session: SessionId,
        name: String,
    },
    WindowRenamed {
        window: WindowId,
        name: String,
    },
    BlockRenamed {
        block: BlockId,
        name: String,
    },
    ClientRenamed {
        client: ClientId,
        name: String,
    },
    /// The window's layout or zoom changed: a block was added or closed,
    /// or a block was zoomed or unzoomed. It follows the
    /// [`Event::BlockCreated`] or [`Event::BlockClosed`] that caused it;
    /// closing the whole window sends none.
    LayoutChanged {
        window: WindowId,
        /// `None` once the window has no blocks.
        layout: Option<Layout>,
        zoomed: Option<BlockId>,
    },
}

/// How a block's program ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    /// It exited with this code.
    Code(i32),
    /// This signal ended it (Unix only).
    Signal(i32),
}

/// The progress a program shows, as ConEmu's OSC 9;4 reports it; the
/// numbers are percentages, at most 100.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// None shown.
    Cleared,
    Normal(u8),
    Error(u8),
    /// Busy, with no known amount.
    Indeterminate,
    Paused(u8),
}

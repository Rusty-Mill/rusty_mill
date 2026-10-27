//! Changes to the model, in the order they happened.

use crate::{BlockId, ClientId, SessionId, Size, WindowId};

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
    ClientConnected {
        client: ClientId,
    },
    ClientDisconnected {
        client: ClientId,
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
}

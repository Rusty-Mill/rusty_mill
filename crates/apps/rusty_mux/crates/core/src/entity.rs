//! The model's entities. Only [`Mux`](crate::Mux) builds or changes them, so
//! their cross-references stay consistent.

use crate::{BlockId, ClientId, Error, Layout, Principal, SessionId, Transport, WindowId};

/// A terminal size in cells. Both dimensions are at least 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Size {
    // Never zero; [`crate::layout`] builds sizes from non-zero parts.
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

impl Size {
    /// Checks that neither dimension is zero.
    pub fn new(cols: u16, rows: u16) -> Result<Self, Error> {
        if cols == 0 || rows == 0 {
            return Err(Error::ZeroSize { cols, rows });
        }
        Ok(Self { cols, rows })
    }

    pub const fn cols(self) -> u16 {
        self.cols
    }

    pub const fn rows(self) -> u16 {
        self.rows
    }
}

/// A named group of windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub(crate) id: SessionId,
    pub(crate) name: String,
    pub(crate) windows: Vec<WindowId>,
}

impl Session {
    pub fn id(&self) -> SessionId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Windows in creation order.
    pub fn windows(&self) -> &[WindowId] {
        &self.windows
    }
}

/// A named group of blocks within a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub(crate) id: WindowId,
    pub(crate) session: SessionId,
    pub(crate) name: String,
    pub(crate) blocks: Vec<BlockId>,
    /// Holds exactly `blocks`; `None` while there are none.
    pub(crate) layout: Option<Layout>,
    pub(crate) zoomed: Option<BlockId>,
}

impl Window {
    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn session(&self) -> SessionId {
        self.session
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Blocks in creation order.
    pub fn blocks(&self) -> &[BlockId] {
        &self.blocks
    }

    /// Where its blocks go; `None` while it has none.
    pub fn layout(&self) -> Option<&Layout> {
        self.layout.as_ref()
    }

    /// The block shown alone, over the whole window, if any.
    pub fn zoomed(&self) -> Option<BlockId> {
        self.zoomed
    }
}

/// One PTY and the terminal that parses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub(crate) id: BlockId,
    pub(crate) window: WindowId,
    pub(crate) name: String,
    pub(crate) size: Size,
    pub(crate) lease: Option<ClientId>,
}

impl Block {
    pub fn id(&self) -> BlockId {
        self.id
    }

    pub fn window(&self) -> WindowId {
        self.window
    }

    /// Empty until a client names it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The PTY size.
    pub fn size(&self) -> Size {
        self.size
    }

    /// The client allowed to write input, if any.
    pub fn lease(&self) -> Option<ClientId> {
        self.lease
    }
}

/// One client connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    pub(crate) id: ClientId,
    pub(crate) name: String,
    pub(crate) attached: Option<BlockId>,
    pub(crate) principal: Principal,
    pub(crate) transport: Transport,
}

impl Client {
    pub fn id(&self) -> ClientId {
        self.id
    }

    /// Empty until a client names it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The block whose output this client receives, if any.
    pub fn attached(&self) -> Option<BlockId> {
        self.attached
    }

    /// Who the server verified this client to be.
    pub fn principal(&self) -> &Principal {
        &self.principal
    }

    /// How its current connection reaches the server.
    pub fn transport(&self) -> Transport {
        self.transport
    }
}

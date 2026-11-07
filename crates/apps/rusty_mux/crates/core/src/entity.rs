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
    /// One of `windows`; `None` only while there are none.
    pub(crate) active: Option<WindowId>,
    /// Whether the server made up `name` and nobody has renamed it since.
    pub(crate) generated: bool,
}

impl Session {
    pub fn id(&self) -> SessionId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the name is one the server made up and nobody has renamed
    /// the session since: unlike [`crate::is_generated`], which reads only
    /// the spelling, this knows where the name came from.
    pub fn generated(&self) -> bool {
        self.generated
    }

    /// Windows in creation order.
    pub fn windows(&self) -> &[WindowId] {
        &self.windows
    }

    /// The window clients show for the session unless told otherwise.
    pub fn active(&self) -> Option<WindowId> {
        self.active
    }
}

/// A named group of blocks within a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub(crate) id: WindowId,
    pub(crate) session: SessionId,
    pub(crate) name: String,
    pub(crate) blocks: Vec<BlockId>,
    /// Holds `blocks` but the detached ones; `None` while none is placed.
    pub(crate) layout: Option<Layout>,
    /// One of the blocks in `layout`, never a detached one.
    pub(crate) zoomed: Option<BlockId>,
    /// One of the blocks in `layout`, the zoomed one if any; `None` only
    /// while none is placed.
    pub(crate) focused: Option<BlockId>,
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

    /// Where its blocks go; `None` while none is placed.
    pub fn layout(&self) -> Option<&Layout> {
        self.layout.as_ref()
    }

    /// Its blocks in no layout, in creation order: they keep running
    /// unseen until moved back in.
    pub fn detached(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.blocks
            .iter()
            .copied()
            .filter(|b| !self.layout.as_ref().is_some_and(|l| l.contains(*b)))
    }

    /// The block shown alone, over the whole window, if any.
    pub fn zoomed(&self) -> Option<BlockId> {
        self.zoomed
    }

    /// The block in focus, shared by every client; `None` only while none
    /// is placed. The attach client follows it: a person types into it.
    pub fn focused(&self) -> Option<BlockId> {
        self.focused
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
    pub(crate) owner: Option<Owner>,
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

    /// Who controls the block, if it was launched owned (C46).
    pub fn owner(&self) -> Option<&Owner> {
        self.owner.as_ref()
    }
}

/// An owned block's owner (C46): the client that launched it, and that
/// client's principal. Once the client is gone for good the block is
/// orphaned: it keeps the principal, which alone may close it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub(crate) principal: Principal,
    pub(crate) client: Option<ClientId>,
}

impl Owner {
    /// The owning client; `None` once the block is orphaned.
    pub fn client(&self) -> Option<ClientId> {
        self.client
    }

    /// Who the owning client was verified to be.
    pub fn principal(&self) -> &Principal {
        &self.principal
    }

    /// The owning client is gone for good.
    pub fn is_orphaned(&self) -> bool {
        self.client.is_none()
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

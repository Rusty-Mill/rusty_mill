//! Why a [`Mux`](crate::Mux) operation was refused. A refused operation changes nothing.

use std::fmt;

use crate::{Axis, BlockId, ClientId, SessionId, Side, WindowId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    UnknownSession(SessionId),
    UnknownWindow(WindowId),
    UnknownBlock(BlockId),
    UnknownClient(ClientId),
    /// A name is longer than [`MAX_NAME`](crate::MAX_NAME) bytes.
    NameTooLong {
        len: usize,
    },
    /// A block with this id already exists.
    BlockExists(BlockId),
    ZeroSize {
        cols: u16,
        rows: u16,
    },
    /// The client must be attached to the block first.
    NotAttached {
        client: ClientId,
        block: BlockId,
    },
    /// Another client holds the block's input lease.
    LeaseHeld {
        block: BlockId,
        holder: ClientId,
    },
    /// The client does not hold the block's input lease.
    NoLease {
        client: ClientId,
        block: BlockId,
    },
    /// The block is too small along that side's axis to split: each part
    /// and the divider need a cell.
    NoRoom {
        block: BlockId,
        side: Side,
    },
    /// Splitting the block would nest the layout deeper than
    /// [`MAX_DEPTH`](crate::MAX_DEPTH).
    TooDeep(BlockId),
    /// No split along the axis holds the block, so there is none to resize.
    NoSplit {
        block: BlockId,
        axis: Axis,
    },
    /// The two blocks are in different windows; a block moves only
    /// within its own.
    OtherWindow {
        block: BlockId,
        other: BlockId,
    },
    /// The block is detached: it is in no layout, so nothing goes beside it.
    Detached(BlockId),
    /// A block cannot go beside itself.
    SameBlock(BlockId),
    /// The window shows blocks, so a block placed in it needs one to go
    /// beside.
    Placed(WindowId),
    /// The session has no place `index` for the window: places count from
    /// 0 and stop before its number of windows.
    NoPlace {
        window: WindowId,
        index: usize,
    },
    /// A [`Principal`](crate::Principal) with an empty id, or a field
    /// longer than [`MAX_NAME`](crate::MAX_NAME) bytes.
    BadPrincipal,
    /// The session's name was chosen, not generated, so
    /// [`Mux::name_session`](crate::Mux::name_session) leaves it.
    NameChosen(SessionId),
    /// Another client owns the block (C46), so only it may change it.
    NotOwner {
        client: ClientId,
        block: BlockId,
    },
    /// The block's owner is gone for good (C46): nobody may type into it
    /// or change it, and only the owner's principal may close it.
    Orphaned(BlockId),
    /// The block already has an owner.
    Owned(BlockId),
    /// The block has no owner, so nobody grants its lease.
    Unowned(BlockId),
    /// The block's input is the owner's: its lease needs the owner's
    /// grant (C77).
    InputGated(BlockId),
    /// A grant must end after now and at most [`MAX_GRANT_MS`](crate::MAX_GRANT_MS)
    /// from now.
    BadGrant {
        block: BlockId,
        until: u64,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSession(id) => write!(f, "unknown session {id}"),
            Self::UnknownWindow(id) => write!(f, "unknown window {id}"),
            Self::UnknownBlock(id) => write!(f, "unknown block {id}"),
            Self::UnknownClient(id) => write!(f, "unknown client {id}"),
            Self::BadPrincipal => write!(
                f,
                "a principal needs an id, and each field at most {} bytes",
                crate::MAX_NAME
            ),
            Self::BlockExists(id) => write!(f, "block {id} already exists"),
            Self::NameTooLong { len } => write!(
                f,
                "name of {len} bytes is longer than {} bytes",
                crate::MAX_NAME
            ),
            Self::ZeroSize { cols, rows } => write!(f, "size {cols}x{rows} has a zero dimension"),
            Self::NotAttached { client, block } => {
                write!(f, "client {client} is not attached to block {block}")
            }
            Self::LeaseHeld { block, holder } => {
                write!(f, "block {block} input lease is held by client {holder}")
            }
            Self::NoLease { client, block } => {
                write!(
                    f,
                    "client {client} does not hold the input lease of block {block}"
                )
            }
            Self::NoRoom { block, side } => {
                write!(f, "block {block} is too small to split {}", side.name())
            }
            Self::TooDeep(block) => write!(
                f,
                "block {block} is nested {} splits deep already",
                crate::MAX_DEPTH
            ),
            Self::NoSplit { block, axis } => {
                write!(f, "no {} split holds block {block}", axis.name())
            }
            Self::OtherWindow { block, other } => {
                write!(f, "blocks {block} and {other} are in different windows")
            }
            Self::Detached(block) => write!(f, "block {block} is detached from the layout"),
            Self::SameBlock(block) => write!(f, "block {block} cannot go beside itself"),
            Self::Placed(window) => {
                write!(f, "window {window} shows blocks: name one to go beside")
            }
            Self::NameChosen(id) => {
                write!(f, "session {id} already has a chosen name")
            }
            Self::NoPlace { window, index } => {
                write!(f, "no place {index} for window {window} in its session")
            }
            Self::NotOwner { client, block } => {
                write!(f, "block {block} is owned by a client other than {client}")
            }
            Self::Orphaned(block) => write!(f, "block {block} is orphaned: its owner is gone"),
            Self::Owned(block) => write!(f, "block {block} already has an owner"),
            Self::Unowned(block) => write!(f, "block {block} has no owner to grant its lease"),
            Self::InputGated(block) => {
                write!(
                    f,
                    "input_gated: block {block} takes input only with its owner's grant"
                )
            }
            Self::BadGrant { block, until } => write!(
                f,
                "a grant on block {block} must end after now and within 24 h, not at {until}"
            ),
        }
    }
}

impl std::error::Error for Error {}

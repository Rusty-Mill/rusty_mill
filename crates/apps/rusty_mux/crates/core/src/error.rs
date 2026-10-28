//! Why a [`Mux`](crate::Mux) operation was refused. A refused operation changes nothing.

use std::fmt;

use crate::{BlockId, ClientId, SessionId, Side, WindowId};

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
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSession(id) => write!(f, "unknown session {id}"),
            Self::UnknownWindow(id) => write!(f, "unknown window {id}"),
            Self::UnknownBlock(id) => write!(f, "unknown block {id}"),
            Self::UnknownClient(id) => write!(f, "unknown client {id}"),
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
        }
    }
}

impl std::error::Error for Error {}

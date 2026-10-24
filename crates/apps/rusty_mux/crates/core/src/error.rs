//! Why a [`Mux`](crate::Mux) operation was refused. A refused operation changes nothing.

use std::fmt;

use crate::{BlockId, ClientId, SessionId, WindowId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    UnknownSession(SessionId),
    UnknownWindow(WindowId),
    UnknownBlock(BlockId),
    UnknownClient(ClientId),
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
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSession(id) => write!(f, "unknown session {id}"),
            Self::UnknownWindow(id) => write!(f, "unknown window {id}"),
            Self::UnknownBlock(id) => write!(f, "unknown block {id}"),
            Self::UnknownClient(id) => write!(f, "unknown client {id}"),
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
        }
    }
}

impl std::error::Error for Error {}

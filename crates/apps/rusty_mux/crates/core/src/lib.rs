//! rusty_mux's pure domain model, with no I/O and no async runtime.
//!
//! The server owns one PTY per [`Block`]. A [`Window`] groups blocks and a
//! [`Session`] groups windows. A [`Client`] is one connection, attached to at
//! most one block. Each window keeps a [`Layout`], a split tree of its
//! blocks, and may zoom one of them; clients draw it. Each block takes input from a single writer: the client
//! holding its input lease. Every entity has a name, empty unless given,
//! that any client may change.
//!
//! [`Mux`] owns the state. Every change it makes is queued as an [`Event`]
//! that the server drains with [`Mux::take_events`] and fans out.
//!
//! ```
//! use rusty_mux_core::{Event, Mux, Principal, Size, Transport};
//!
//! let mut mux = Mux::new();
//! let session = mux.create_session("work")?;
//! let window = mux.create_window(session, "main")?;
//! let block = mux.create_block(window, Size::new(80, 24)?)?;
//! let client = mux.connect(Principal::unix(1000, None)?, Transport::Socket);
//! mux.attach(client, block)?;
//! mux.acquire_lease(client, block)?;
//! mux.authorize_input(client, block)?;
//! assert!(mux.take_events().contains(&Event::LeaseChanged { block, holder: Some(client) }));
//! # Ok::<(), rusty_mux_core::Error>(())
//! ```

mod entity;
mod error;
mod event;
mod id;
mod identity;
mod layout;
mod mux;

pub use entity::{Block, Client, Session, Size, Window};
pub use error::Error;
pub use event::{Event, ExitStatus, Progress, Selection};
pub use id::{BlockId, ClientId, EntityId, SessionId, WindowId};
pub use identity::{Principal, Transport};
pub use layout::{Axis, Layout, MAX_DEPTH, ParseError, Rect, Share, Side};
pub use mux::{MAX_NAME, Mux};

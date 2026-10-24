//! rusty_mux's pure domain model, with no I/O and no async runtime.
//!
//! The server owns one PTY per [`Block`]. A [`Window`] groups blocks and a
//! [`Session`] groups windows. A [`Client`] is one connection, attached to at
//! most one block. Splits and tabs are drawn by clients, so the server keeps
//! no pane tree. Each block takes input from a single writer: the client
//! holding its input lease.
//!
//! [`Mux`] owns the state. Every change it makes is queued as an [`Event`]
//! that the server drains with [`Mux::take_events`] and fans out.
//!
//! ```
//! use rusty_mux_core::{Event, Mux, Size};
//!
//! let mut mux = Mux::new();
//! let session = mux.create_session("work");
//! let window = mux.create_window(session, "main")?;
//! let block = mux.create_block(window, Size::new(80, 24)?)?;
//! let client = mux.connect();
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
mod mux;

pub use entity::{Block, Client, Session, Size, Window};
pub use error::Error;
pub use event::Event;
pub use id::{BlockId, ClientId, SessionId, WindowId};
pub use mux::Mux;

//! rusty_mux's wire protocol, with no I/O.
//!
//! A frame is a `u32` big-endian length, then a `u8` message type, then the
//! message's fields. The length counts the type byte and the fields. Each
//! field is a LEB128 varint key, `id << 1 | kind`, followed by either a
//! varint (kind 0) or a varint length and that many bytes (kind 1). Readers
//! skip field ids they do not know, so a peer can add fields without
//! breaking an older one. A new message type is gated by a [`Features`] bit
//! that both peers set in [`Message::Hello`] and [`Message::Welcome`], so an
//! unknown message type is an error rather than something to skip.
//!
//! ```
//! use rusty_mux_proto::{Features, Message, decode, encode};
//!
//! let frame = encode(&Message::Hello { features: Features::NONE })?;
//! let (message, used) = decode(&frame)?.expect("a whole frame");
//! assert_eq!(message, Message::Hello { features: Features::NONE });
//! assert_eq!(used, frame.len());
//! # Ok::<(), rusty_mux_proto::Error>(())
//! ```

mod error;
mod field;
mod frame;
mod message;
mod varint;

pub use error::Error;
pub use frame::{MAX_FRAME, decode, encode};
pub use message::{Features, Message};

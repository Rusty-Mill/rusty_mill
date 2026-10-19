//! A Protobuf message layer for hand-written schemas, on [`rusty_protobuf`].
//!
//! A message is a plain struct with `Default`. Its schema is two functions: [`Message::encode`]
//! writes the fields in field-number order (omitting proto3 defaults, writing `Option`s that are
//! `Some`), and [`Message::merge_field`] is a `match` on [`Field::number`] that reads one
//! occurrence into the struct. Everything else (merging a whole buffer, `decode`,
//! `encode_to_vec`) is provided.
//!
//! The merge rules are the format's: a scalar's last occurrence wins, a sub-message's
//! occurrences merge, a repeated field's occurrences append, a oneof's last member wins. Unknown
//! fields are skipped. [`field`] has the helpers sub-messages and maps need.
//!
//! The well-known types live in [`wkt`] and `google.rpc.Status`/`ErrorInfo` in [`rpc`].

mod message;

pub mod field;
pub mod rpc;
pub mod wkt;

pub use message::{Message, Name, Result};
pub use rusty_protobuf::{Error, Field, Reader, Writer, DEFAULT_DEPTH_LIMIT};

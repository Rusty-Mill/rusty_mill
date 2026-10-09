//! A small FlatBuffers runtime: read and build the binary format without code generation.
//!
//! The reader never panics on hostile input: every access is bounds-checked and returns
//! [`Error`]. The builder writes back to front like the reference implementation, so buffers
//! it produces are readable by any FlatBuffers reader.
//!
//! Schemas are hand-written on top of [`Table`] and [`TableBuilder`]: a field is addressed by
//! its slot (its declaration index in the schema), a union is a `u8` type slot followed by an
//! offset slot.

mod builder;
mod error;
mod reader;
mod scalar;

pub use builder::{Builder, Offset, TableBuilder};
pub use error::Error;
pub use reader::{has_identifier, Table, Vector};
pub use scalar::Scalar;

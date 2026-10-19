//! A small Protobuf runtime: read and write the binary wire format without code generation.
//!
//! The reader never panics on hostile input: every access is bounds-checked and returns
//! [`Error`]. It borrows from the input and allocates nothing, whatever a length field claims.
//! The writer produces the same bytes as the reference implementations when fields are written
//! in field-number order.
//!
//! Schemas are hand-written on top: a decoder loops over a [`Reader`] and matches on
//! [`Field::number`]; an encoder calls [`Writer`] methods. Two rules the schema owns:
//!
//! - **Unknown fields** are yielded like any other; a decoder that does not match them skips
//!   them, and they are not kept for re-encoding.
//! - **Repeated occurrences** of a field arrive as separate [`Field`]s. A scalar's last
//!   occurrence wins, a sub-message's occurrences merge, a repeated field's occurrences
//!   concatenate (a packed run and unpacked elements may be mixed).
//!
//! Groups (wire types 3 and 4) are rejected with [`Error::Group`].

mod error;
mod reader;
mod varint;
mod wire;
mod writer;

pub use error::Error;
pub use reader::{Field, Fixed32s, Fixed64s, Reader, Value, Varints};
pub use varint::{unzigzag32, unzigzag64, zigzag32, zigzag64};
pub use wire::{WireType, DEFAULT_DEPTH_LIMIT, MAX_FIELD_NUMBER};
pub use writer::Writer;

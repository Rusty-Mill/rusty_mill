//! A from-scratch JSON library for Rust.
//!
//! `no_std` + `alloc` by default; enable the `std` feature (on by default)
//! for `Display`/`Error` impls and other std-only ergonomics.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod de;
mod error;
mod formatter;
mod macros;
mod map;
mod number;
mod parser;
mod ser;
mod serde_support;
mod value;
mod value_de;
mod value_ser;

pub use error::{Category, Error};
pub use formatter::{CharEscape, CompactFormatter, Formatter, PrettyFormatter};
pub use map::{
    Entry, IntoIter, IntoValues, Iter, IterMut, Keys, Map, OccupiedEntry, VacantEntry, Values,
    ValuesMut,
};
pub use number::Number;
pub use value::Value;

/// Shorthand for `Result<T, Error>`, matching this crate's error type.
pub type Result<T> = core::result::Result<T, Error>;

pub use de::{from_slice, from_str, StreamDeserializer};
pub use ser::{
    to_string, to_string_pretty, to_string_with_formatter, to_vec, to_vec_pretty, Compound,
    Serializer,
};
pub use value_de::from_value;
pub use value_ser::to_value;

#[cfg(feature = "std")]
pub use de::from_reader;
#[cfg(feature = "std")]
pub use ser::{to_writer, to_writer_pretty};

/// Not public API. Re-exports used by the [`json!`] macro's expansion so it
/// works from downstream crates without them needing `extern crate alloc`.
#[doc(hidden)]
pub mod __private {
    pub use alloc::vec;
}

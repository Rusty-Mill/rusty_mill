//! A from-scratch JSON library for Rust.
//!
//! `no_std` + `alloc` by default; enable the `std` feature (on by default)
//! for `Display`/`Error` impls and other std-only ergonomics.
//!
//! The `serde` feature (on by default) adds `from_str`/`to_string` and
//! friends for arbitrary `Deserialize`/`Serialize` types, `from_value`/
//! `to_value`, and `Value`'s own `serde::Serialize`/`Deserialize` impls (for
//! driving `Value` through other serde data formats). With `serde` disabled
//! (`default-features = false`), this crate has **no dependency on `serde`
//! at all** -- [`Value`] parsing (`s.parse::<Value>()`) and writing
//! ([`Value::to_json_string`]/[`Value::to_json_string_pretty`]) still work,
//! for callers that want JSON support without pulling in `serde`.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(feature = "serde")]
mod de;
mod error;
mod escape;
mod formatter;
mod macros;
mod map;
mod number;
mod parser;
#[cfg(feature = "serde")]
mod ser;
#[cfg(feature = "serde")]
mod serde_support;
mod value;
#[cfg(feature = "serde")]
mod value_de;
mod value_io;
#[cfg(feature = "serde")]
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

#[cfg(feature = "serde")]
pub use de::{from_slice, from_str, StreamDeserializer};
#[cfg(feature = "serde")]
pub use ser::{
    to_string, to_string_pretty, to_string_with_formatter, to_vec, to_vec_pretty, Compound,
    Serializer,
};
#[cfg(feature = "serde")]
pub use value_de::from_value;
#[cfg(feature = "serde")]
pub use value_ser::to_value;

#[cfg(all(feature = "std", feature = "serde"))]
pub use de::from_reader;
#[cfg(all(feature = "std", feature = "serde"))]
pub use ser::{to_writer, to_writer_pretty};

/// Not public API. Re-exports used by the [`json!`] macro's expansion so it
/// works from downstream crates without them needing `extern crate alloc`.
#[doc(hidden)]
pub mod __private {
    pub use alloc::vec;
}

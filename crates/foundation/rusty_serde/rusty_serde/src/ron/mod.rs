//! A second, RON-inspired format built directly on the [`crate::Serializer`]
//! / [`crate::Deserializer`] traits, existing to prove those traits (and
//! `#[derive(Serialize, Deserialize)]` on top of them) don't know or care
//! which concrete syntax they're driven by - the exact same derived impls
//! that round-trip through [`crate::json`] round-trip through here too,
//! with a genuinely different wire shape: unquoted struct field names,
//! bracket choice that varies by data shape (`[...]` for sequences,
//! `{...}` for maps/structs, `(...)` for tuples/newtypes), map keys that
//! aren't restricted to strings, and bare-identifier enum tags instead of
//! JSON's `{"Variant": ...}` wrapping.
//!
//! It is *not* an implementation of the real [RON](https://github.com/ron-rs/ron)
//! format - no extensions, no spec compliance - just RON-flavored enough to
//! make the point.

mod de;
mod error;
mod ser;

pub use de::{from_str, Deserializer};
pub use error::Error;
pub use ser::{to_string, Serializer};

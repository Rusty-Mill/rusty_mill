//! A compact JSON format built directly on the [`crate::Serializer`] /
//! [`crate::Deserializer`] traits - no external JSON crate involved.

mod de;
mod error;
mod ser;

/// Re-exported for discoverability - `Value` itself is format-agnostic
/// (see [`crate::value`]), but this is where most people go looking for a
/// `serde_json::Value` equivalent.
pub use crate::value::Value;
pub use de::{from_str, Deserializer};
pub use error::Error;
pub use ser::{to_string, Serializer};

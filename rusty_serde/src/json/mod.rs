//! A compact JSON format built directly on the [`crate::Serializer`] /
//! [`crate::Deserializer`] traits - no external JSON crate involved.

mod de;
mod error;
mod ser;

pub use de::{from_str, Deserializer};
pub use error::Error;
pub use ser::{to_string, Serializer};

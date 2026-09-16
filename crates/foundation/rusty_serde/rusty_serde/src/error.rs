use std::fmt::Display;

/// Minimal error contract shared by every [`crate::Serializer`] and
/// [`crate::Deserializer`] implementation.
///
/// Kept intentionally small (a single `custom` constructor) so that a format
/// crate only needs to provide `std::error::Error + Display` and a way to
/// build itself from an arbitrary message.
pub trait Error: std::error::Error + Sized {
    fn custom<T>(msg: T) -> Self
    where
        T: Display;
}

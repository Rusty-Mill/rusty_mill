#![no_std]
#![deny(missing_docs)]

//! # `rusty_err`
//!
//! A `#![no_std]` + `alloc` sovereign error trait, context extension,
//! and proc-macro derive library for the **Rusty Mill** ecosystem.

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use core::any::Any;
use core::error::Error as CoreError;
use core::fmt::{self, Debug, Display};

pub use rusty_err_derive::Error;

/// Sovereign Error trait.
pub trait Error: Debug + Display {
    /// Returns lower-level cause of this error if available.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

/// Bridges every [`core::error::Error`] implementor — the wider Rust
/// ecosystem's errors (`std::io::Error`, `serde_json::Error`, ...) — into a
/// sovereign [`Error`], so they can be used directly as `#[from]`/`#[source]`
/// fields with [`derive@Error`] and boxed by [`BoxError`].
///
/// This bridges one hop of the chain: [`Error::source`] on a bridged type
/// returns `None` rather than recursing into the wrapped error's own
/// [`core::error::Error::source`], since a `&dyn core::error::Error` cannot
/// be safely re-coerced into `&dyn Error` without unsafe code. The immediate
/// cause is always preserved; only chains more than one foreign hop deep are
/// truncated.
impl<E: CoreError + 'static> Error for E {}

/// Extension trait for adding contextual error messages to Result types.
pub trait Context<T, E> {
    /// Contextualizes an error result with a static context string.
    fn context(self, msg: &'static str) -> Result<T, String>;

    /// Contextualizes an error result using a closure returning a context string.
    fn with_context<F: FnOnce() -> String>(self, f: F) -> Result<T, String>;
}

impl<T, E: Display> Context<T, E> for Result<T, E> {
    fn context(self, msg: &'static str) -> Result<T, String> {
        self.map_err(|e| format!("{}: {}", msg, e))
    }

    fn with_context<F: FnOnce() -> String>(self, f: F) -> Result<T, String> {
        self.map_err(|e| format!("{}: {}", f(), e))
    }
}

/// Type-erasure helper implemented for every sovereign [`Error`], giving
/// [`BoxError`] a way to downcast back to the concrete type it was built
/// from.
///
/// Bounded by `Send + Sync` (on top of [`Error`] itself) so that
/// `Box<dyn AnyError>` — and therefore [`BoxError`] — is `Send + Sync`
/// unconditionally, matching `anyhow::Error`'s own guarantee. This is what
/// lets `BoxError` satisfy the `Send` futures that `#[async_trait]` methods
/// require by default.
trait AnyError: Error + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

impl<E: Error + Send + Sync + 'static> AnyError for E {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

/// A boxed, type-erased [`Error`], analogous to `anyhow::Error`.
///
/// Unlike [`Context`], which immediately formats the source error into a
/// `String` and discards it, `BoxError` boxes the original error behind one
/// type while preserving [`Display`], [`Debug`], [`Error::source`] chaining,
/// and the ability to downcast back to the concrete type.
///
/// `BoxError` is `Send + Sync` unconditionally (the boxed error must be
/// `Send + Sync` too — see [`BoxError::new`]), so it can be used as the
/// error type of `Send` futures, e.g. in `#[async_trait]` method signatures.
pub struct BoxError {
    inner: Box<dyn AnyError>,
}

impl BoxError {
    /// Boxes any sovereign [`Error`] into a type-erased `BoxError`.
    pub fn new<E: Error + Send + Sync + 'static>(err: E) -> Self {
        BoxError {
            inner: Box::new(err),
        }
    }

    /// Returns the lower-level cause of this error, if available.
    pub fn source(&self) -> Option<&(dyn Error + 'static)> {
        Error::source(&*self.inner)
    }

    /// Returns a reference to the boxed error if it is of type `E`.
    pub fn downcast_ref<E: Error + 'static>(&self) -> Option<&E> {
        self.inner.as_any().downcast_ref()
    }

    /// Returns a mutable reference to the boxed error if it is of type `E`.
    pub fn downcast_mut<E: Error + 'static>(&mut self) -> Option<&mut E> {
        self.inner.as_any_mut().downcast_mut()
    }

    /// Consumes the `BoxError`, returning the concrete error if it is of
    /// type `E`, or the `BoxError` unchanged otherwise.
    pub fn downcast<E: Error + 'static>(self) -> Result<E, Self> {
        if self.inner.as_any().is::<E>() {
            Ok(*self
                .inner
                .into_any()
                .downcast::<E>()
                .expect("type checked above"))
        } else {
            Err(self)
        }
    }
}

impl Debug for BoxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&self.inner, f)
    }
}

impl Display for BoxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.inner, f)
    }
}

impl<E: Error + Send + Sync + 'static> From<E> for BoxError {
    fn from(err: E) -> Self {
        BoxError::new(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn context_attachment() {
        let res: Result<(), &'static str> = Err("file not found");
        let ctx_res = res.context("Failed to load config");
        assert_eq!(
            ctx_res.unwrap_err(),
            "Failed to load config: file not found"
        );
    }

    #[derive(Debug)]
    struct Leaf;

    impl Display for Leaf {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "leaf failure")
        }
    }

    impl Error for Leaf {}

    #[derive(Debug)]
    struct Wrapper {
        cause: Leaf,
    }

    impl Display for Wrapper {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "wrapper failure")
        }
    }

    impl Error for Wrapper {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.cause)
        }
    }

    #[test]
    fn box_error_preserves_display_debug_and_source() {
        let boxed = BoxError::new(Wrapper { cause: Leaf });
        assert_eq!(boxed.to_string(), "wrapper failure");
        assert!(format!("{:?}", boxed).contains("Wrapper"));
        assert!(boxed.source().is_some());
    }

    #[test]
    fn box_error_downcasts() {
        let boxed = BoxError::new(Wrapper { cause: Leaf });
        assert!(boxed.downcast_ref::<Leaf>().is_none());
        assert!(boxed.downcast_ref::<Wrapper>().is_some());

        let boxed = BoxError::new(Leaf);
        match boxed.downcast::<Wrapper>() {
            Ok(_) => panic!("should not downcast to the wrong type"),
            Err(boxed) => {
                let leaf = boxed.downcast::<Leaf>().expect("downcast to original type");
                assert_eq!(leaf.to_string(), "leaf failure");
            }
        }
    }

    #[derive(Debug)]
    struct ForeignError;

    impl Display for ForeignError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "foreign failure")
        }
    }

    impl CoreError for ForeignError {}

    #[test]
    fn core_error_bridge() {
        // `ForeignError` only implements `core::error::Error`; the blanket
        // bridge impl should make it usable as a sovereign `Error` too,
        // including being boxed by `BoxError`.
        let boxed: BoxError = ForeignError.into();
        assert_eq!(boxed.to_string(), "foreign failure");
    }

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn box_error_is_send_and_sync() {
        // Regression test for the `#[async_trait]` blocker from
        // https://github.com/baileyrd/rusty_err/issues/4: `BoxError` must be
        // `Send + Sync` unconditionally, matching `anyhow::Error`.
        assert_send_sync::<BoxError>();
    }
}

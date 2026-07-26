#![no_std]
#![deny(missing_docs)]

//! # `rusty_err`
//!
//! A `#![no_std]` + `alloc` sovereign error trait, context extension,
//! and proc-macro derive library for the **Rusty Mill** ecosystem.

extern crate alloc;

use alloc::format;
use alloc::string::String;
use core::fmt::{Debug, Display};

/// Sovereign Error trait.
pub trait Error: Debug + Display {
    /// Returns lower-level cause of this error if available.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_attachment() {
        let res: Result<(), &'static str> = Err("file not found");
        let ctx_res = res.context("Failed to load config");
        assert_eq!(ctx_res.unwrap_err(), "Failed to load config: file not found");
    }
}

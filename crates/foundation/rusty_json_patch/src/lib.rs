//! JSON Pointer (RFC 6901), JSON Patch (RFC 6902) and JSON Merge Patch
//! (RFC 7386) over [`rusty_json::Value`].
//!
//! `no_std` + `alloc`; the `std` feature (on by default) adds
//! `std::error::Error` for [`Error`]. One dependency, `rusty_json`, and no
//! external ones (ADR-0002 Tier S).
//!
//! - [`Pointer`]: a parsed pointer, with escaping handled once.
//! - [`Patch`] / [`Op`]: a patch document; [`Patch::apply`] is atomic
//!   (all operations or none).
//! - [`diff`]: the patch that turns one document into another.
//! - [`merge_patch`]: RFC 7386, the "overlay this object" form.
//!
//! ```
//! use rusty_json::{json, Value};
//! use rusty_json_patch::{diff, Patch};
//!
//! let mut doc = json!({"a": {"b": 1}, "list": [1, 2]});
//! let target = json!({"a": {"b": 2}, "list": [1, 2, 3]});
//! let patch = diff(&doc, &target);
//! patch.apply(&mut doc).unwrap();
//! assert_eq!(doc, target);
//!
//! let wire: Value = patch.to_value();
//! assert_eq!(Patch::from_value(&wire).unwrap(), patch);
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod diff;
mod error;
mod merge;
mod patch;
mod pointer;

pub use diff::diff;
pub use error::Error;
pub use merge::merge_patch;
pub use patch::{Op, Patch};
pub use pointer::Pointer;

/// Shorthand for `Result<T, Error>`.
pub type Result<T> = core::result::Result<T, Error>;

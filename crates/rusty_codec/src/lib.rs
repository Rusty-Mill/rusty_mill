#![no_std]
#![deny(missing_docs)]

//! # `rusty_codec`
//!
//! A `#![no_std]` + `alloc` sovereign TOML configuration parser (`rusty_toml`)
//! and binary buffer serialization engine (`rusty_bincode`) for the **Rusty Mill** ecosystem.

extern crate alloc;

pub mod binary;
pub mod toml;

pub use binary::{deserialize, serialize};
pub use toml::TomlValue;

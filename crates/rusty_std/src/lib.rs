#![cfg_attr(not(test), no_std)]
#![deny(missing_docs)]

//! # `rusty_std`
//!
//! A `#![no_std]` + `alloc` sovereign standard library for the **Rusty Mill** ecosystem.
//! Provides std-compatible interfaces directly on top of Level 0 raw kernel interfaces:
//! `rusty_libc` (Linux raw syscalls) and `rusty_win32` (Windows FFI).

extern crate alloc;

pub mod env;
pub mod error;
pub mod fs;
pub mod io;
pub mod net;
pub mod path;
pub mod process;
pub mod sync;
pub mod time;

pub use error::{Error, Result};

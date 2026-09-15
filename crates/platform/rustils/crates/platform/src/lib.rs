//! # platform — the portable api layer of rustils
//!
//! This crate defines the OS-agnostic trait surface and types that every
//! backend implements and every consumer programs against. It performs no
//! I/O and contains no `unsafe`; all OS interaction lives in the backend
//! crates (`platform-linux`, `platform-windows`) and all test doubles in
//! `platform-mock`.
//!
//! Governing document: `docs/rfc-v2.md`. Design requirements for this
//! surface are RFC §5; the consumer gate (§3) governs what may be added.

#![forbid(unsafe_code)]

pub mod error;
pub mod events;
pub mod fs;
pub mod net;
pub mod process;
pub mod pty;
pub mod security;
pub mod term;
pub mod tun;

pub use error::{ErrorKind, OsCode, PlatformError};

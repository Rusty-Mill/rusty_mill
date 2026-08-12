//! # platform-async — the async api layer of rustils_async
//!
//! Async counterpart to rustils' `platform` crate (pinned as a git
//! dependency rather than forked — see the root `Cargo.toml` — so the
//! two share one data model: [`platform::process::Command`],
//! [`platform::process::ExitStatus`], [`platform::error::PlatformError`]
//! are the same types on both the sync and async sides, per Rusty-Mill
//! AKB Foundation Principle #4, "async-first, sync-complete").
//!
//! Governed by:
//! - rustils' own `docs/rfc-v2.md` for the sync types and layering
//!   conventions this crate mirrors.
//! - `rusty_foundation_akb` ADR-0160 for why this crate stays
//!   domain-scoped (built on [`reactor_core`]'s primitives) rather than
//!   one blanket "async" trait other crates inherit from.
//! - this repo's own `docs/adr/0001-native-async-rustils.md` for why it
//!   exists at all, and why it starts with the `process` domain only.
//!
//! No I/O, no unsafe — same discipline as `platform`.

#![forbid(unsafe_code)]

pub mod process;

pub use platform::error::{ErrorKind, OsCode, PlatformError, Result};

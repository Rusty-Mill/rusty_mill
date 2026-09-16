//! Byte cursor primitives for the RDP codec.
//!
//! RDP mixes endianness: the transport layers (TPKT, X.224) are big-endian
//! while the RDP structures layered on top are little-endian — exactly what
//! [`rusty_wire`] was built for, so this module re-exports its `Reader`/
//! `Writer` rather than reimplementing the same bounds-checked cursor here.
//! [`crate::error::Error`] mirrors `rusty_wire::Error`'s two variants field
//! for field, so the `From` impl in `error.rs` lets every `?` call site here
//! keep working unchanged.

pub use rusty_wire::{Reader, Writer};

//! ASN.1 BER/DER TLV encoder and decoder for Rusty Mill, built on [`rusty_wire`].
//!
//! Formerly bundled together with an unrelated sovereign RAG/Q&A engine
//! under this crate's portmanteau name ("ans" + "der"); that engine now
//! lives in its own crate, `rusty_rag`.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

pub mod der;

pub use der::*;

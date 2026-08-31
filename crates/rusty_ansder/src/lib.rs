//! Sovereign AI Retrieval-Augmented Generation (RAG) & Question Answering Engine and ASN.1 DER Parser for Rusty Mill.
//!
//! Exposes:
//! - [`rag`]: Sovereign AI retrieval and Q&A engine connecting search and LLM inference.
//! - [`der`]: ASN.1 BER/DER TLV encoder and decoder built on [`rusty_wire`].

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

pub mod der;
pub mod rag;

pub use der::*;
pub use rag::*;

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]

//! # `rusty_wiremock`
//!
//! A blocking `std::net` HTTP mock server for Rusty Mill test suites.
//!
//! The [`canned`] module, behind the `std` feature, answers a fixed
//! sequence of canned responses. `rusty_proxmox`, `rusty_opnsense`,
//! `rusty_fedora`, and `rusty_homelab_mcp`'s client tests drive their
//! clients against it (each used to carry its own identical copy under
//! `tests/support/`). Without `std` the crate is empty.

#[cfg(feature = "std")]
pub mod canned;
